use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use axum::Router;
use axum::extract::ws::Message as AxumMessage;
use axum::extract::ws::WebSocketUpgrade;
use axum::http::HeaderMap;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use futures::SinkExt as _;
use futures::StreamExt as _;
use http_body_util::BodyExt as _;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::ResolvedService;
use leptoskit_config::ServiceConfig;
use leptoskit_config::ServiceManifest;

use super::*;

fn write(root: &Path, path: &str, contents: &str) {
	let path = root.join(path);
	std::fs::create_dir_all(path.parent().unwrap()).unwrap();
	std::fs::write(path, contents).unwrap();
}

/// A workspace with an app and an analytics service, whose manifest
/// declares a port, a database, a volume, and a binding.
fn project() -> tempfile::TempDir {
	let dir = tempfile::tempdir().unwrap();
	let root = dir.path();
	write(
		root,
		"Cargo.toml",
		"[workspace]\nmembers = [\"app\", \"analytics\"]\nresolver = \"2\"\n",
	);
	write(
		root,
		"app/Cargo.toml",
		"[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\nssr = \
		 []\n",
	);
	write(
		root,
		"app/src/main.rs",
		"fn main() { println!(\"app ssr={}\", cfg!(feature = \"ssr\")); }\n",
	);
	write(
		root,
		"analytics/Cargo.toml",
		"[package]\nname = \"analytics\"\nversion = \"0.1.0\"\nedition = \
		 \"2021\"\n\n[[bin]]\nname = \"analytics\"\nrequired-features = \
		 [\"service\"]\n\n[features]\nservice = []\n\n[package.metadata.leptoskit.service]\nname \
		 = \"analytics\"\nport = 3100\npublic = true\nfeatures = [\"service\"]\ndatabase = \
		 true\npool = 4\nvolumes = \
		 [\"exports\"]\n\n[package.metadata.leptoskit.service.bind]\nANALYTICS_PUBLIC_PATH = \
		 \"{public_path}\"\nANALYTICS_URL = \"{url}\"\n",
	);
	write(root, "analytics/src/lib.rs", "");
	write(root, "analytics/src/main.rs", "fn main() {}\n");
	write(
		root,
		"leptoskit.toml",
		"[app]\nname = \"app\"\n\n[db]\ndialect = \"sqlite\"\n\n[services.analytics]\npackage = \
		 \"analytics\"\nroutes = [\"/_analytics\"]\n\n[services.billing]\nurl = \
		 \"https://billing.example\"\nbind = { BILLING_API = \"{url}/api\" }\n",
	);
	dir
}

fn load(root: &Path) -> (LeptoskitConfig, CargoMetadata) {
	let config = LeptoskitConfig::load_from(root).expect("config");
	let metadata = crate::wasm::cargo_metadata(root).expect("metadata");
	(config, metadata)
}

#[test]
fn resolves_and_lists_services_without_secrets() {
	let dir = project();
	let root = dir.path();
	let (config, metadata) = load(root);
	let services = resolve(&config, root, &metadata, false).expect("resolve");

	assert_eq!(services.len(), 2);
	let analytics = &services[0];
	assert!(matches!(&analytics.code, Code::Workspace { package, .. } if package == "analytics"));
	assert_eq!(analytics.settings.port, Some(3100));
	assert_eq!(analytics.settings.features, ["service"]);
	assert_eq!(
		analytics.settings.trust,
		leptoskit_config::ServiceTrust::FirstParty
	);
	assert_eq!(services[1].code, Code::Remote);
	assert_eq!(
		services[1].settings.trust,
		leptoskit_config::ServiceTrust::Isolated
	);

	let listing = serde_json::to_value(listing(&services, &config, root)).unwrap();
	insta::assert_json_snapshot!("services_listing", listing);
}

#[test]
fn a_missing_package_names_the_table() {
	let dir = project();
	let root = dir.path();
	write(
		root,
		"leptoskit.toml",
		"[app]\nname = \"app\"\n\n[services.search]\npackage = \"search\"\nport = 3200\n",
	);
	let (config, metadata) = load(root);
	let error = resolve(&config, root, &metadata, false).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("services.search.package names `search`"),
		"{error}"
	);
}

#[test]
fn a_table_named_unlike_its_manifest_fails() {
	let dir = project();
	let root = dir.path();
	write(
		root,
		"leptoskit.toml",
		"[app]\nname = \"app\"\n\n[services.stats]\npackage = \"analytics\"\n",
	);
	let (config, metadata) = load(root);
	let error = resolve(&config, root, &metadata, false).unwrap_err();
	assert!(
		error.to_string().contains("names the service `analytics`")
			&& error.to_string().contains("[services.analytics]"),
		"{error}"
	);
}

#[test]
fn a_service_without_a_port_fails_after_its_manifest_is_read() {
	let dir = project();
	let root = dir.path();
	write(
		root,
		"leptoskit.toml",
		"[app]\nname = \"app\"\n\n[services.app-two]\npackage = \"app\"\n",
	);
	let (config, metadata) = load(root);
	let error = resolve(&config, root, &metadata, false).unwrap_err();
	assert!(error.to_string().contains("has no port"), "{error}");
}

#[test]
fn builds_the_app_and_its_services_in_one_invocation() {
	let dir = project();
	let root = dir.path();
	let (config, metadata) = load(root);
	let services = resolve(&config, root, &metadata, false).unwrap();
	let options = CargoOptions {
		features: vec!["ssr".to_owned()],
		target_directory: Some(root.join("target")),
		..CargoOptions::default()
	};
	let built = build(
		root,
		&metadata,
		Some(&AppBuild {
			package: "app",
			options: &options,
		}),
		&services,
		false,
	)
	.expect("build");

	let app = built.app.expect("the app's server");
	let output = Command::new(&app).output().unwrap();
	assert_eq!(
		String::from_utf8_lossy(&output.stdout).trim(),
		"app ssr=true"
	);
	let analytics = built.services[0].binary.as_ref().expect("analytics");
	assert!(analytics.ends_with(format!("analytics{}", std::env::consts::EXE_SUFFIX)));
	assert_eq!(
		built.services[1].binary, None,
		"a remote service builds nothing"
	);
}

#[test]
fn features_are_qualified_by_package() {
	assert_eq!(qualify("app", "ssr"), "app/ssr");
	assert_eq!(qualify("app", "leptos/islands"), "leptos/islands");
}

#[test]
fn installed_services_are_fetched_by_one_manifest() {
	let crate_source = ServiceSource::Crate {
		name: "acme-search".into(),
		version: Some("0.4".into()),
	};
	let git_source = ServiceSource::Git {
		url: "https://example.com/billing".into(),
		package: "billing".into(),
		reference: Some(GitReference::Tag("v1".into())),
	};
	let (search, search_dependency) = dependency_of(&crate_source).unwrap();
	let (billing, billing_dependency) = dependency_of(&git_source).unwrap();
	let manifest = stub_manifest(&[
		("search", &search, &search_dependency),
		("billing", &billing, &billing_dependency),
	]);
	insta::assert_snapshot!("fetch_manifest", manifest);
	let parsed: toml::Table = toml::from_str(&manifest).expect("valid TOML");
	assert_eq!(
		parsed["dependencies"]["service-billing"]["tag"].as_str(),
		Some("v1")
	);
}

fn built(name: &str, raw: &str, manifest: Option<&ServiceManifest>, binary: bool) -> BuiltService {
	let table: ServiceConfig = toml::from_str(raw).unwrap();
	let settings = ResolvedService::merge(name, &table, manifest).unwrap();
	let code = if binary {
		Code::Workspace {
			package: name.to_owned(),
			manifest: format!("/{name}/Cargo.toml").into(),
		}
	} else {
		Code::Remote
	};
	BuiltService {
		service: Service { settings, code },
		binary: binary.then(|| format!("/bin/{name}").into()),
	}
}

#[test]
fn prepares_each_service_and_the_app() {
	let dir = project();
	let root = dir.path();
	let (config, _) = load(root);
	let manifest = ServiceManifest {
		name: Some("analytics".into()),
		port: Some(3100),
		public: Some(true),
		database: Some(true),
		pool: Some(4),
		volumes: Some(vec!["exports".into()]),
		env: [("RETENTION_DAYS".into(), "90".into())].into(),
		bind: [("ANALYTICS_PATH".into(), "{public_path}".into())].into(),
		..ServiceManifest::default()
	};
	let services = [
		built("analytics", "package = 'analytics'", Some(&manifest), true),
		built(
			"billing",
			"url = 'https://billing.example/'\nbind = { BILLING_API = '{url}/api' }",
			None,
			false,
		),
	];
	let tokens: BTreeMap<String, String> = [("analytics".into(), "secret".into())].into();
	let session = run::Session {
		config: &config,
		root,
		public_origin: "http://127.0.0.1:3000".into(),
		app_url: "http://127.0.0.1:41000".into(),
		app_database_url: None,
		tokens: &tokens,
	};
	let prepared = run::prepare(&services, &session).expect("prepare");

	let launch = &prepared.launches[0];
	assert_eq!(launch.address.to_string(), "127.0.0.1:3100");
	let env: BTreeMap<_, _> = launch.env.iter().cloned().collect();
	assert_eq!(env["LEPTOSKIT_SERVICE_NAME"], "analytics");
	assert_eq!(env["LEPTOSKIT_SERVICE_ADDR"], "127.0.0.1:3100");
	assert_eq!(env["LEPTOSKIT_SERVICE_TOKEN"], "secret");
	assert_eq!(env["LEPTOSKIT_PUBLIC_ORIGIN"], "http://127.0.0.1:3000");
	assert_eq!(env["LEPTOSKIT_APP_URL"], "http://127.0.0.1:41000");
	assert_eq!(env["LEPTOSKIT_SERVICE_DATABASE_POOL"], "4");
	assert_eq!(env["RETENTION_DAYS"], "90");
	let database = sqlite_path(root, "analytics");
	assert_eq!(
		env["DATABASE_URL"],
		format!("sqlite://{}", database.display())
	);
	assert!(database.parent().unwrap().is_dir());
	let exports = dev_dir(root, "analytics").join("exports");
	assert_eq!(
		env["LEPTOSKIT_SERVICE_VOLUME_EXPORTS"],
		exports.display().to_string()
	);
	assert!(exports.is_dir());

	let app: BTreeMap<_, _> = prepared.app_env.iter().cloned().collect();
	assert_eq!(
		app["LEPTOSKIT_SERVICE_ANALYTICS_URL"],
		"http://127.0.0.1:3100"
	);
	assert_eq!(app["LEPTOSKIT_SERVICE_ANALYTICS_TOKEN"], "secret");
	assert_eq!(app["ANALYTICS_PATH"], "/_leptoskit/services/analytics");
	assert_eq!(
		app["LEPTOSKIT_SERVICE_BILLING_URL"],
		"https://billing.example"
	);
	assert_eq!(app["BILLING_API"], "https://billing.example/api");
	assert_eq!(prepared.launches.len(), 1, "a remote service isn't started");
}

#[test]
fn tokens_are_random_and_only_for_processes() {
	let services = [
		built("a", "package = 'a'\nport = 1", None, true),
		built("b", "url = 'https://b.example'", None, false),
	];
	let first = run::tokens(&services).unwrap();
	let second = run::tokens(&services).unwrap();
	assert_eq!(first.keys().collect::<Vec<_>>(), ["a"]);
	assert_eq!(first["a"].len(), 64);
	assert_ne!(first["a"], second["a"]);
}

#[test]
fn routes_pick_the_most_specific_prefix_by_segment() {
	let services = [
		built(
			"analytics",
			"package = 'a'\nport = 3100\npublic = true\nroutes = ['/_analytics']",
			None,
			true,
		),
		built(
			"search",
			"crate = 's'\nport = 3200\npublic = true",
			None,
			true,
		),
		built("private", "package = 'p'\nport = 3300", None, true),
	];
	let routes = proxy::Routes::of(&services);
	let target = |path: &str| {
		routes
			.route(path)
			.map(|route| (route.service.as_str(), route.isolated))
	};

	assert_eq!(
		target("/_leptoskit/services/analytics"),
		Some(("analytics", false))
	);
	assert_eq!(
		target("/_leptoskit/services/analytics/api/capture"),
		Some(("analytics", false))
	);
	assert_eq!(target("/_analytics/script.js"), Some(("analytics", false)));
	assert_eq!(target("/_analyticsx"), None);
	assert_eq!(
		target("/_leptoskit/services/search/live"),
		Some(("search", true))
	);
	assert_eq!(target("/_leptoskit/services/private/api/x"), None);
	assert_eq!(target("/"), None);
}

/// A service with an echo, a stream, a cookie-setting route, and a
/// WebSocket echo, on an OS-picked port.
async fn upstream_service() -> u16 {
	let prefix = "/_leptoskit/services/analytics";
	let router = Router::new()
		.route(
			&format!("{prefix}/echo"),
			get(|headers: HeaderMap| {
				async move {
					let cookie = headers
						.get(header::COOKIE)
						.and_then(|value| value.to_str().ok())
						.unwrap_or("none")
						.to_owned();
					(
						[(header::SET_COOKIE, "session=service")],
						format!("service saw cookie: {cookie}"),
					)
				}
			}),
		)
		.route(
			&format!("{prefix}/stream"),
			get(|| {
				async {
					let (sender, receiver) =
						tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(1);
					tokio::spawn(async move {
						let _ = sender.send(Ok("first".to_owned())).await;
						tokio::time::sleep(Duration::from_millis(300)).await;
						let _ = sender.send(Ok("second".to_owned())).await;
					});
					axum::body::Body::from_stream(tokio_stream(receiver))
				}
			}),
		)
		.route(
			&format!("{prefix}/ws"),
			get(|upgrade: WebSocketUpgrade| async move { upgrade.on_upgrade(echo) }),
		);
	serve(router).await
}

/// Answers each text frame with `echo <text>`.
async fn echo(mut socket: axum::extract::ws::WebSocket) {
	while let Some(Ok(message)) = socket.recv().await {
		if let AxumMessage::Text(text) = message {
			let _ = socket
				.send(AxumMessage::Text(format!("echo {text}").into()))
				.await;
		}
	}
}

fn tokio_stream(
	mut receiver: tokio::sync::mpsc::Receiver<Result<String, std::io::Error>>,
) -> impl futures::Stream<Item = Result<String, std::io::Error>> {
	futures::stream::poll_fn(move |context| receiver.poll_recv(context))
}

async fn serve(router: Router) -> u16 {
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let port = listener.local_addr().unwrap().port();
	tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
	port
}

async fn proxy_for(trust: &str) -> u16 {
	let service = upstream_service().await;
	let app = serve(Router::new().fallback(|headers: HeaderMap| {
		async move {
			let cookie = headers
				.get(header::COOKIE)
				.and_then(|value| value.to_str().ok())
				.unwrap_or("none")
				.to_owned();
			format!("app saw cookie: {cookie}").into_response()
		}
	}))
	.await;
	let services = [built(
		"analytics",
		&format!("package = 'a'\nport = {service}\npublic = true\ntrust = '{trust}'"),
		None,
		true,
	)];
	let router = proxy::router(
		proxy::Routes::of(&services),
		std::net::SocketAddr::from(([127, 0, 0, 1], app)),
	);
	serve(router).await
}

async fn get_text(port: u16, path: &str) -> (HeaderMap, String) {
	let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
		.build_http::<axum::body::Body>();
	let request = axum::http::Request::get(format!("http://127.0.0.1:{port}{path}"))
		.header(header::COOKIE, "session=app")
		.body(axum::body::Body::empty())
		.unwrap();
	let response = client.request(request).await.unwrap();
	let headers = response.headers().clone();
	let body = response.into_body().collect().await.unwrap().to_bytes();
	(headers, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn the_proxy_sends_service_paths_to_the_service_and_the_rest_to_the_app() {
	let proxy = proxy_for("first-party").await;
	let (headers, body) = get_text(proxy, "/_leptoskit/services/analytics/echo").await;
	assert_eq!(body, "service saw cookie: session=app");
	assert_eq!(headers[header::SET_COOKIE], "session=service");
	let (_, body) = get_text(proxy, "/dashboard").await;
	assert_eq!(body, "app saw cookie: session=app");
}

#[tokio::test]
async fn an_isolated_service_never_sees_or_sets_the_apps_cookies() {
	let proxy = proxy_for("isolated").await;
	let (headers, body) = get_text(proxy, "/_leptoskit/services/analytics/echo").await;
	assert_eq!(body, "service saw cookie: none");
	assert!(headers.get(header::SET_COOKIE).is_none());
	let (_, body) = get_text(proxy, "/").await;
	assert_eq!(
		body, "app saw cookie: session=app",
		"the app keeps its cookies"
	);
}

#[tokio::test]
async fn the_proxy_streams_bodies_as_they_arrive() {
	let proxy = proxy_for("first-party").await;
	let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
		.build_http::<axum::body::Body>();
	let response = client
		.get(
			format!("http://127.0.0.1:{proxy}/_leptoskit/services/analytics/stream")
				.parse()
				.unwrap(),
		)
		.await
		.unwrap();
	let mut body = response.into_body();
	let started = std::time::Instant::now();
	let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
	assert_eq!(&first[..], b"first");
	assert!(
		started.elapsed() < Duration::from_millis(250),
		"the first chunk waited for the second"
	);
	let second = body.frame().await.unwrap().unwrap().into_data().unwrap();
	assert_eq!(&second[..], b"second");
}

#[tokio::test]
async fn the_proxy_hands_websockets_through() {
	let proxy = proxy_for("isolated").await;
	let (mut socket, _) = tokio_tungstenite::connect_async(format!(
		"ws://127.0.0.1:{proxy}/_leptoskit/services/analytics/ws"
	))
	.await
	.expect("upgrade through the proxy");
	socket
		.send(tungstenite::Message::text("hello"))
		.await
		.unwrap();
	let reply = socket.next().await.unwrap().unwrap();
	assert_eq!(reply.into_text().unwrap(), "echo hello");
}

#[tokio::test]
async fn an_unreachable_upstream_is_a_bad_gateway() {
	let services = [built(
		"analytics",
		"package = 'a'\nport = 1\npublic = true",
		None,
		true,
	)];
	let router = proxy::router(
		proxy::Routes::of(&services),
		std::net::SocketAddr::from(([127, 0, 0, 1], 1)),
	);
	let proxy = serve(router).await;
	let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
		.build_http::<axum::body::Body>();
	let response = client
		.get(
			format!("http://127.0.0.1:{proxy}/_leptoskit/services/analytics/x")
				.parse()
				.unwrap(),
		)
		.await
		.unwrap();
	assert_eq!(response.status(), axum::http::StatusCode::BAD_GATEWAY);
	let body = response.into_body().collect().await.unwrap().to_bytes();
	assert!(String::from_utf8_lossy(&body).contains("services.analytics"));
}

#[test]
fn readiness_waits_for_a_200() {
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let port = runtime.block_on(serve(
		Router::new().route("/readyz", get(|| async { "ok" })),
	));
	assert!(run::ready(std::net::SocketAddr::from((
		[127, 0, 0, 1],
		port
	))));
	let missing = runtime.block_on(serve(Router::new()));
	assert!(!run::ready(std::net::SocketAddr::from((
		[127, 0, 0, 1],
		missing
	))));
}

#[test]
fn service_databases_stay_out_of_leptoskits_own_names() {
	assert_eq!(
		postgres_database("leptoskit", "analytics"),
		"leptoskit_service_analytics"
	);
	// `admin` would otherwise be `leptoskit_admin`, the managed superuser.
	assert_eq!(
		postgres_database("leptoskit", "admin"),
		"leptoskit_service_admin"
	);
	assert_eq!(
		postgres_database("app__feat_x", "event-log"),
		"app__feat_x_service_event_log"
	);
	let long = postgres_database(&"a".repeat(60), "analytics");
	assert_eq!(long.len(), 63);
	assert_ne!(long, postgres_database(&"a".repeat(60), "cards"));
}

#[test]
fn a_service_inherits_only_what_a_program_needs() {
	use std::ffi::OsString;

	let vars = [
		("PATH", "/bin"),
		("HOME", "/home/me"),
		("LC_ALL", "C"),
		("RUST_LOG", "debug"),
		("SystemRoot", "C:\\Windows"),
		("DATABASE_URL", "postgres://app"),
		("AWS_SECRET_ACCESS_KEY", "secret"),
		("LEPTOSKIT_SERVICE_ANALYTICS_TOKEN", "token"),
	]
	.map(|(name, value)| (OsString::from(name), OsString::from(value)));
	let names: Vec<String> = run::inherited(vars)
		.into_iter()
		.map(|(name, _)| name.into_string().unwrap())
		.collect();
	assert_eq!(names, ["PATH", "HOME", "LC_ALL", "RUST_LOG", "SystemRoot"]);
}

#[test]
fn a_taken_port_stops_a_service_before_it_starts() {
	let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
	let address = taken.local_addr().unwrap();
	let prepared = run::Prepared {
		launches: vec![run::Launch {
			name: "analytics".into(),
			binary: "/nonexistent/analytics".into(),
			address,
			env: Vec::new(),
		}],
		app_env: Vec::new(),
	};
	let error = run::start(&prepared, &[], Duration::from_secs(1))
		.err()
		.expect("a taken port")
		.to_string();
	assert!(
		error.contains("services.analytics") && error.contains("already listening"),
		"{error}"
	);
	drop(taken);
}

/// `SIGTERM` reaches the child, which stops on its own; one that ignores it
/// is killed once the grace period passes.
#[cfg(unix)]
#[test]
fn stopping_sends_sigterm_then_sigkill() {
	let dir = tempfile::tempdir().unwrap();
	let marker = dir.path().join("terminated");
	let mut polite = Command::new("sh")
		.arg("-c")
		.arg(format!(
			"trap 'touch {}; exit 0' TERM; while :; do sleep 0.05; done",
			marker.display()
		))
		.spawn()
		.unwrap();
	let mut stubborn = Command::new("sh")
		.arg("-c")
		.arg("trap '' TERM; while :; do sleep 0.05; done")
		.spawn()
		.unwrap();
	// Let both shells install their traps.
	std::thread::sleep(Duration::from_millis(300));
	let started = std::time::Instant::now();
	crate::server::stop_gracefully(
		&mut [&mut polite, &mut stubborn],
		Duration::from_millis(500),
	);
	let elapsed = started.elapsed();
	assert!(marker.exists(), "the trap ran, so SIGTERM arrived");
	assert!(elapsed >= Duration::from_millis(500), "{elapsed:?}");
	assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
	assert!(polite.try_wait().unwrap().is_some());
	assert!(stubborn.try_wait().unwrap().is_some());
}

/// What `leptoskit dev` does for `examples/analytics` on Postgres, for real:
/// builds the service with its `postgres` feature, gives it its own role and
/// database on the development server, starts it, and sends captures through
/// the proxy, which land in the service's database and nowhere else.
///
/// Ignored by default: it builds the example's service, and needs Postgres
/// binaries (`LEPTOSKIT_POSTGRES_BIN`, `PGBIN`, or the path). Run it with
/// `cargo test -p leptoskit_cli --all-features analytics_runs_on_postgres --
/// --ignored`.
#[test]
#[ignore = "builds examples/analytics and runs Postgres"]
fn analytics_runs_on_postgres_in_a_database_of_its_own() {
	use leptoskit_db::Db;
	use leptoskit_db::Read as _;
	use leptoskit_db::Statement;
	use leptoskit_db::Value;

	let Some(binaries) = crate::db::dev_server::test_binaries() else {
		return;
	};
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
	let workspace = workspace.canonicalize().unwrap();
	let port = std::net::TcpListener::bind("127.0.0.1:0")
		.unwrap()
		.local_addr()
		.unwrap()
		.port();
	// The app's project: a git repository on its main branch, holding the
	// development Postgres and the service's files, apart from the example.
	let project = tempfile::tempdir().unwrap();
	let root = project.path();
	let status = crate::git::command(root)
		.args(["init", "--quiet", "--initial-branch=main"])
		.env("GIT_CONFIG_GLOBAL", root.join("no-global-config"))
		.env("GIT_CONFIG_NOSYSTEM", "1")
		.status()
		.unwrap();
	assert!(status.success());
	let config = LeptoskitConfig::from_toml(
		&format!(
			"[app]\nname = \"analytics-app\"\n\n[db]\ndialect = \"postgres\"\ndev_server = \
			 \"embedded\"\n\n[services.analytics]\npackage = \"analytics_service\"\nfeatures = \
			 [\"service\", \"postgres\"]\nport = {port}\n"
		),
		root,
	)
	.unwrap();

	let metadata = crate::wasm::cargo_metadata(&workspace).unwrap();
	let services = resolve(&config, &workspace, &metadata, false).unwrap();
	let built = build(&workspace, &metadata, None, &services, false).unwrap();
	let app = crate::db::Project::of(&config, root.to_path_buf()).unwrap();
	let server = crate::db::dev_server::start(
		&app,
		crate::db::dev_server::Source::Embedded,
		Some(&binaries),
	)
	.unwrap();
	assert!(server.url.contains("/analytics_app?"), "{}", server.url);

	let tokens = run::tokens(&built.services).unwrap();
	let session = run::Session {
		config: &config,
		root,
		public_origin: "http://127.0.0.1:3000".into(),
		app_url: "http://127.0.0.1:9".into(),
		app_database_url: Some(server.url.clone()),
		tokens: &tokens,
	};
	let prepared = run::prepare(&built.services, &session).unwrap();
	let launch = &prepared.launches[0];
	let service_url = launch
		.env
		.iter()
		.find(|(name, _)| name == env::DATABASE_URL)
		.map(|(_, url)| url.clone())
		.unwrap();
	let parsed = url::Url::parse(&service_url).unwrap();
	assert_eq!(parsed.username(), "analytics_app_service_analytics");
	assert_eq!(parsed.path(), "/analytics_app_service_analytics");
	let mut running = run::start(
		&prepared,
		&[(env::DRAIN_MS.to_owned(), "0".to_owned())],
		Duration::from_secs(5),
	)
	.unwrap();

	let runtime = tokio::runtime::Runtime::new().unwrap();
	let statuses = runtime.block_on(async {
		let app = serve(Router::new().fallback(|| async { "the app" })).await;
		let proxy = serve(proxy::router(
			proxy::Routes::of(&built.services),
			std::net::SocketAddr::from(([127, 0, 0, 1], app)),
		))
		.await;
		let client =
			hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
				.build_http::<axum::body::Body>();
		let mut statuses = Vec::new();

		for event in ["signup", "clicked_buy", "clicked_buy"] {
			let request = axum::http::Request::post(format!(
				"http://127.0.0.1:{proxy}/_leptoskit/services/analytics/api/capture"
			))
			.header(header::CONTENT_TYPE, "application/json")
			.body(axum::body::Body::from(
				serde_json::json!({"event": event, "distinct_id": "visitor-1", "properties": {}})
					.to_string(),
			))
			.unwrap();
			statuses.push(client.request(request).await.unwrap().status().as_u16());
		}

		statuses
	});
	running.stop();
	assert_eq!(statuses, [200, 200, 200]);

	let first = |url: &str, sql: &str| -> Option<Value> {
		crate::db::runtime().unwrap().block_on(async {
			let db = Db::connect(url).await.ok()?;
			let rows = db
				.fetch(Statement {
					sql: sql.to_owned(),
					args: vec![],
				})
				.await
				.ok()?;
			rows.into_iter().next()?.into_iter().next()
		})
	};
	let as_admin =
		crate::db::dev_server::with_database(&server.url, "analytics_app_service_analytics")
			.unwrap();
	assert_eq!(
		first(&as_admin, "SELECT count(*) FROM events"),
		Some(Value::Int(3)),
		"the captures land in the service's database"
	);
	assert_eq!(
		first(&server.url, "SELECT to_regclass('events') IS NULL"),
		Some(Value::Bool(true)),
		"and not in the app's"
	);
	let reach_app = crate::db::dev_server::with_database(&service_url, "analytics_app").unwrap();
	assert_eq!(
		first(&reach_app, "SELECT 1"),
		None,
		"the service's role can't connect to the app's database"
	);
}
