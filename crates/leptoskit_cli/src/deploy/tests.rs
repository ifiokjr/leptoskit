//! `leptoskit generate deploy` against projects written by hand: every
//! target with no database, SQLite, and Postgres, pinned file by file.

use std::fmt::Write as _;
use std::path::Path;

use leptoskit_config::LeptoskitConfig;

use super::Project;
use super::Target;
use super::generate;
use super::generate_with;

const SERVER: &str = r"
use leptoskit::server::Readiness;

fn main() {
	let readiness = Readiness::new();
}
";

#[derive(Clone, Copy, Debug)]
pub(crate) enum Db {
	None,
	Sqlite,
	Postgres,
}

impl Db {
	fn name(self) -> &'static str {
		match self {
			Self::None => "no_db",
			Self::Sqlite => "sqlite",
			Self::Postgres => "postgres",
		}
	}
}

fn write(root: &Path, file: &str, contents: &str) {
	let path = root.join(file);
	std::fs::create_dir_all(path.parent().unwrap()).unwrap();
	std::fs::write(path, contents).unwrap();
}

/// A workspace like the scaffold's: the server package at the root, the
/// client in a member. The Postgres project also signs people in.
fn project(root: &Path, db: Db) -> LeptoskitConfig {
	let mut config = String::from(
		"schema_version = 0\n\n[app]\nname = \"notes\"\nversion = \"0.1.0\"\nrender = \
		 \"ssr\"\n\n[serve]\naddress = \"127.0.0.1\"\nport = 3000\nprobes = \
		 true\n\n[build]\nclient_package = \"notes_app\"\n",
	);

	match db {
		Db::None => {}
		Db::Sqlite => config.push_str("\n[db]\ndialect = \"sqlite\"\n"),
		Db::Postgres => {
			config.push_str(
				"\n[db]\ndialect = \"postgres\"\n\n[auth]\nkeys_env = \
				 \"AUTH_ENCRYPTION_KEYS\"\n\n[auth.mail]\nfrom = \"Notes \
				 <hello@notes.example>\"\nsmtp_url_env = \"SMTP_URL\"\n",
			);
		}
	}

	write(root, "leptoskit.toml", &config);
	write(
		root,
		"Cargo.toml",
		"[package]\nname = \"notes\"\nversion = \"0.1.0\"\n\n[workspace]\nmembers = \
		 [\"notes_app\"]\n",
	);
	write(root, "src/main.rs", SERVER);
	write(
		root,
		"notes_app/Cargo.toml",
		"[package]\nname = \"notes_app\"\nversion = \"0.1.0\"\n\n[lib]\ncrate-type = [\"rlib\", \
		 \"cdylib\"]\n",
	);
	LeptoskitConfig::from_toml(&config, root).expect("config")
}

fn with_version_masked() -> insta::Settings {
	let mut settings = insta::Settings::clone_current();
	settings.add_filter(
		&format!(
			"LEPTOSKIT_VERSION={}",
			regex_escape(env!("CARGO_PKG_VERSION"))
		),
		"LEPTOSKIT_VERSION=[version]",
	);
	settings
}

fn regex_escape(text: &str) -> String {
	text.chars()
		.map(|character| {
			if character.is_ascii_alphanumeric() {
				character.to_string()
			} else {
				format!("\\{character}")
			}
		})
		.collect()
}

/// Every target, with every database: the output, then each file.
#[test]
fn every_target_and_database_is_pinned() {
	let _version = with_version_masked().bind_to_scope();
	for target in [
		Target::Docker,
		Target::Compose,
		Target::Fly,
		Target::Kubernetes,
		Target::Systemd,
	] {
		for db in [Db::None, Db::Sqlite, Db::Postgres] {
			let root = tempfile::tempdir().unwrap();
			let config = project(root.path(), db);
			let report = generate(root.path(), &config, target, false).expect("generate");
			let mut pinned = format!("$ leptoskit generate deploy --target {target}\n{report}");

			for (file, _) in target.files(&Project::read(root.path(), &config).unwrap()) {
				let contents = std::fs::read_to_string(root.path().join(&file)).unwrap();

				write!(pinned, "\n===== {file} =====\n{contents}").unwrap();
			}

			insta::assert_snapshot!(format!("{target}_{}", db.name()), pinned);
		}
	}
}

/// Kubernetes with each role its own Deployment, on Postgres with a relay
/// and a replica: the output, then the manifest.
#[test]
fn split_roles_are_pinned() {
	let _version = with_version_masked().bind_to_scope();
	let root = tempfile::tempdir().unwrap();
	project(root.path(), Db::Postgres);
	let toml = std::fs::read_to_string(root.path().join("leptoskit.toml"))
		.unwrap()
		.replace(
			"dialect = \"postgres\"\n",
			"dialect = \"postgres\"\nreplica_url_envs = [\"DATABASE_REPLICA_URL\"]\n",
		)
		+ "\n[rpc.live]\nrelay_url_env = \"NATS_URL\"\n";
	write(root.path(), "leptoskit.toml", &toml);
	let config = LeptoskitConfig::from_toml(&toml, root.path()).unwrap();
	let report = generate_with(root.path(), &config, Target::Kubernetes, false, true).unwrap();
	let manifest = std::fs::read_to_string(root.path().join("kubernetes.yaml")).unwrap();
	insta::assert_snapshot!(
		"kubernetes_split_roles",
		format!(
			"$ leptoskit generate deploy --target kubernetes --split-roles\n{report}\n===== \
			 kubernetes.yaml =====\n{manifest}"
		)
	);
}

#[test]
fn split_roles_need_kubernetes_and_postgres() {
	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::Postgres);
	let error = generate_with(root.path(), &config, Target::Fly, false, true).unwrap_err();
	assert!(error.to_string().contains("--target kubernetes"), "{error}");

	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::Sqlite);
	let error = generate_with(root.path(), &config, Target::Kubernetes, false, true).unwrap_err();
	assert!(error.to_string().contains("needs Postgres"), "{error}");
}

#[test]
fn running_again_changes_nothing_and_says_so() {
	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::Sqlite);
	generate(root.path(), &config, Target::Fly, false).unwrap();
	let fly = std::fs::read_to_string(root.path().join("fly.toml")).unwrap();

	let again = generate(root.path(), &config, Target::Fly, false).unwrap();
	assert!(
		again.changed.is_empty() && again.kept.is_empty(),
		"{again:?}"
	);
	assert!(
		again
			.to_string()
			.starts_with("nothing changed: the fly files are already in place\n\nnext:\n"),
		"{again}"
	);
	assert_eq!(
		std::fs::read_to_string(root.path().join("fly.toml")).unwrap(),
		fly
	);

	// Another target shares the Dockerfile and adds only its own file.
	let compose = generate(root.path(), &config, Target::Compose, false).unwrap();
	assert_eq!(compose.changed, ["created compose.yaml"]);
}

#[test]
fn an_edited_file_is_kept_unless_forced() {
	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::None);
	generate(root.path(), &config, Target::Docker, false).unwrap();
	let dockerfile = root.path().join("Dockerfile");
	let original = std::fs::read_to_string(&dockerfile).unwrap();
	std::fs::write(&dockerfile, "FROM scratch\n").unwrap();

	let kept = generate(root.path(), &config, Target::Docker, false).unwrap();
	assert_eq!(
		kept.kept,
		["kept Dockerfile: it differs from the template; --force replaces it"]
	);
	assert!(kept.to_string().contains("nothing changed"), "{kept}");
	assert_eq!(
		std::fs::read_to_string(&dockerfile).unwrap(),
		"FROM scratch\n"
	);

	let forced = generate(root.path(), &config, Target::Docker, true).unwrap();
	assert_eq!(forced.changed, ["replaced Dockerfile"]);
	assert_eq!(forced.kept, [] as [String; 0]);
	assert_eq!(std::fs::read_to_string(&dockerfile).unwrap(), original);
}

#[test]
fn a_directory_in_the_way_is_an_error() {
	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::None);
	std::fs::create_dir(root.path().join("Dockerfile")).unwrap();
	let error = generate(root.path(), &config, Target::Docker, true).unwrap_err();
	assert_eq!(error.to_string(), "Dockerfile isn't an ordinary file");
}

#[test]
fn a_server_without_probes_is_told_how_to_add_them() {
	let root = tempfile::tempdir().unwrap();
	let mut config = project(root.path(), Db::None);
	config.serve.probes = false;
	write(
		root.path(),
		"src/main.rs",
		"fn main() { /* Readiness readyz */ }\n",
	);
	let report = generate(root.path(), &config, Target::Docker, false).unwrap();
	assert!(
		report.next[0].starts_with("Backend probes are not declared"),
		"{report}"
	);
}

#[test]
fn the_project_is_read_from_its_manifests() {
	let root = tempfile::tempdir().unwrap();
	let mut config = project(root.path(), Db::None);
	write(
		root.path(),
		"rust-toolchain.toml",
		"[toolchain]\nchannel = \"1.96.0\"\n",
	);
	write(
		root.path(),
		"notes_app/Cargo.toml",
		"[package]\nname = \"notes_app\"\n\n[lib]\nname = \"notes_client\"\n",
	);
	config.app.platforms = vec![leptoskit_config::Platform::Ios];
	config.serve.port = 8080;
	let project = Project::read(root.path(), &config).unwrap();
	assert_eq!(project.binary, "notes");

	assert_eq!(project.output_name, "notes_client");
	assert_eq!(project.port, 8080);
	assert_eq!(project.platform, leptoskit_config::Platform::Server);
	assert_eq!(
		project.toolchain_file.as_deref(),
		Some("rust-toolchain.toml")
	);
	assert_eq!(project.rust_version, "1.96.0");
	assert!(project.has_probes);

	// A server-only build skips the wasm toolchain and binaryen, and the
	// toolchain file comes in before anything is built.
	let dockerfile = super::dockerfile(&project);
	assert!(!dockerfile.contains("wasm"), "{dockerfile}");
	assert!(dockerfile.contains("ARG RUST_VERSION=1.96.0\n"));
	assert!(dockerfile.contains("COPY rust-toolchain.toml ./\nCOPY . .\n"));
	assert!(dockerfile.contains("leptoskit build --release --platform server"));
	assert!(dockerfile.contains("EXPOSE 8080\n"));

	// Without a client package, the output name is the app's.
	config.build.client_package = None;
	assert_eq!(
		Project::read(root.path(), &config).unwrap().output_name,
		"notes"
	);
}

#[test]
fn toolchain_channels_pin_the_image_only_when_they_name_a_release() {
	let root = tempfile::tempdir().unwrap();
	let file = root.path().join("rust-toolchain");
	for (contents, expected) in [
		("1.96\n", Some("1.96")),
		("stable\n", None),
		("nightly-2026-10-01\n", None),
		("[toolchain]\nchannel = \"beta\"\n", None),
		("[toolchain]\nprofile = \"minimal\"\n", None),
		("1..2\n", None),
		("\n", None),
	] {
		std::fs::write(&file, contents).unwrap();
		assert_eq!(
			super::toolchain_channel(&file).as_deref(),
			expected,
			"{contents:?}"
		);
	}
	assert_eq!(super::toolchain_channel(&root.path().join("missing")), None);
}

#[test]
fn the_binary_is_the_one_named_after_the_package_or_the_only_one() {
	let root = tempfile::tempdir().unwrap();
	let dir = root.path();
	write(
		dir,
		"Cargo.toml",
		"[package]\nname = \"notes\"\n\n[[bin]]\nname = \"serve\"\npath = \"src/serve.rs\"\n",
	);
	write(dir, "src/serve.rs", "fn main() {}\n");
	let (binary, source) = super::binary(dir, "notes").unwrap();
	assert_eq!(binary, "serve");
	assert_eq!(source, Some(dir.join("src/serve.rs")));

	write(
		dir,
		"Cargo.toml",
		"[package]\nname = \"notes\"\n\n[[bin]]\nname = \"serve\"\n\n[[bin]]\nname = \"notes\"\n",
	);
	let (binary, source) = super::binary(dir, "notes").unwrap();
	assert_eq!(binary, "notes");
	assert_eq!(source, None, "src/bin/notes.rs doesn't exist");

	write(
		dir,
		"Cargo.toml",
		"[package]\nname = \"notes\"\n\n[[bin]]\nname = \"a\"\n\n[[bin]]\nname = \"b\"\n",
	);
	let error = super::binary(dir, "notes").unwrap_err().to_string();
	assert!(
		error.starts_with(
			"the server package `notes` has several binaries and none is called `notes`"
		),
		"{error}"
	);

	write(dir, "Cargo.toml", "[package]\nname = \"notes\"\n");
	std::fs::remove_file(dir.join("src/serve.rs")).unwrap();
	let error = super::binary(dir, "notes").unwrap_err().to_string();
	assert!(
		error.starts_with("the server package `notes` has no binary"),
		"{error}"
	);
}

#[test]
fn what_is_missing_is_named() {
	let root = tempfile::tempdir().unwrap();
	let mut config = project(root.path(), Db::None);

	config.app.render = leptoskit_config::RenderMode::Csr;
	assert!(Project::read(root.path(), &config).is_ok());
	let client = config.build.client_package.take();
	assert!(Project::read(root.path(), &config).is_ok());
	config.build.backend = Some(false);
	config.serve.probes = false;
	let error = Project::read(root.path(), &config).unwrap_err().to_string();
	assert!(
		error.starts_with("build.backend = false selects a static app"),
		"{error}"
	);

	config.build.backend = Some(true);
	config.app.render = leptoskit_config::RenderMode::Islands;
	config.build.client_package = client;
	config.build.client_package = Some("missing_app".into());
	let error = Project::read(root.path(), &config).unwrap_err().to_string();
	assert_eq!(
		error,
		"build.client_package `missing_app` in leptoskit.toml isn't a package in the workspace"
	);

	config.app.name = "missing".into();
	let error = Project::read(root.path(), &config).unwrap_err().to_string();
	assert!(
		error.starts_with(
			"app.name `missing` in leptoskit.toml must name the server package, but there's no \
			 package called `missing` in "
		),
		"{error}"
	);

	write(root.path(), "Cargo.toml", "[package\n");
	let error = Project::read(root.path(), &config).unwrap_err().to_string();
	assert!(error.starts_with("could not parse "), "{error}");

	std::fs::remove_file(root.path().join("Cargo.toml")).unwrap();
	let error = Project::read(root.path(), &config).unwrap_err().to_string();
	assert!(error.starts_with("there's no Cargo.toml at "), "{error}");
}

#[test]
fn probes_depend_on_the_declared_contract_not_source_spelling() {
	let root = tempfile::tempdir().unwrap();
	let mut config = project(root.path(), Db::None);
	write(root.path(), "src/main.rs", "fn main() { server::run(); }\n");
	assert!(Project::read(root.path(), &config).unwrap().has_probes);
	config.serve.probes = false;
	write(
		root.path(),
		"src/main.rs",
		"// TODO: Readiness /readyz\nfn main() {}\n",
	);
	assert!(!Project::read(root.path(), &config).unwrap().has_probes);
}

/// The notes project with services: `analytics`, a public workspace
/// service with a database, a volume, and bindings; `cards`, a private one;
/// and `billing`, which runs elsewhere. `trust` is analytics's.
pub(crate) fn project_with_services(root: &Path, db: Db, trust: &str) -> LeptoskitConfig {
	let db = match db {
		Db::None => String::new(),
		Db::Sqlite => "[db]\ndialect = \"sqlite\"\n\n".to_owned(),
		Db::Postgres => "[db]\ndialect = \"postgres\"\n\n".to_owned(),
	};
	let config = format!(
		"schema_version = 0\n\n[app]\nname = \"notes\"\nversion = \"0.1.0\"\nrender = \
		 \"ssr\"\n\n[serve]\naddress = \"127.0.0.1\"\nport = 3000\nprobes = \
		 true\n\n[build]\nclient_package = \"notes_app\"\n\n{db}[services.analytics]\npackage = \
		 \"analytics_service\"\ntrust = \"{trust}\"\n\n[services.cards]\npackage = \
		 \"cards\"\n\n[services.billing]\nurl = \"https://billing.internal.example/\"\n"
	);
	write(root, "leptoskit.toml", &config);
	write(
		root,
		"Cargo.toml",
		"[package]\nname = \"notes\"\nversion = \"0.1.0\"\nedition = \
		 \"2024\"\n\n[workspace]\nmembers = [\"notes_app\", \"analytics_service\", \"cards\"]\n",
	);
	write(root, "src/main.rs", SERVER);
	write(
		root,
		"notes_app/Cargo.toml",
		"[package]\nname = \"notes_app\"\nversion = \"0.1.0\"\nedition = \
		 \"2024\"\n\n[lib]\ncrate-type = [\"rlib\", \"cdylib\"]\n",
	);
	write(root, "notes_app/src/lib.rs", "");
	write(
		root,
		"analytics_service/Cargo.toml",
		"[package]\nname = \"analytics_service\"\nversion = \"0.1.0\"\nedition = \
		 \"2024\"\n\n[features]\nservice = []\n\n[package.metadata.leptoskit.service]\nname = \
		 \"analytics\"\nport = 3100\npublic = true\nroutes = [\"/_analytics\"]\ndatabase = \
		 true\npool = 4\nfeatures = [\"service\"]\nvolumes = [\"exports\"]\nsecrets = \
		 [\"ANALYTICS_GEOIP_KEY\"]\nenv = { ANALYTICS_RETENTION_DAYS = \"90\" \
		 }\n\n[package.metadata.leptoskit.service.bind]\nANALYTICS_PUBLIC_PATH = \
		 \"{public_path}\"\nANALYTICS_AUTHORIZATION = \"Bearer {token}\"\n",
	);
	write(root, "analytics_service/src/lib.rs", "");
	write(root, "analytics_service/src/main.rs", "fn main() {}\n");
	write(
		root,
		"cards/Cargo.toml",
		"[package]\nname = \"cards\"\nversion = \"0.1.0\"\nedition = \
		 \"2024\"\n\n[package.metadata.leptoskit.service]\nname = \"cards\"\nport = 3200\n",
	);
	write(root, "cards/src/main.rs", "fn main() {}\n");
	LeptoskitConfig::from_toml(&config, root).expect("config")
}

/// Docker, Compose, and Kubernetes with services, on both databases.
#[test]
fn services_are_pinned() {
	let _version = with_version_masked().bind_to_scope();

	for target in [Target::Docker, Target::Compose, Target::Kubernetes] {
		for db in [Db::Sqlite, Db::Postgres] {
			let root = tempfile::tempdir().unwrap();
			// Kubernetes refuses an isolated public service; see below.
			let trust = if target == Target::Kubernetes {
				"first-party"
			} else {
				"isolated"
			};
			let config = project_with_services(root.path(), db, trust);
			let report = generate(root.path(), &config, target, false).expect("generate");
			let mut pinned = format!("$ leptoskit generate deploy --target {target}\n{report}");

			for (file, _) in target.files(&Project::read(root.path(), &config).unwrap()) {
				let contents = std::fs::read_to_string(root.path().join(&file)).unwrap();

				write!(pinned, "\n===== {file} =====\n{contents}").unwrap();
			}

			insta::assert_snapshot!(format!("services_{target}_{}", db.name()), pinned);
		}
	}
}

/// A service's database is a SQLite file on a volume of its own when the
/// app has no `[db]` at all, as `examples/analytics` does: the containers'
/// root filesystems are read-only.
#[test]
fn a_service_database_without_an_app_database_is_on_a_volume() {
	let root = tempfile::tempdir().unwrap();
	let config = project_with_services(root.path(), Db::None, "first-party");
	generate(root.path(), &config, Target::Compose, false).expect("compose");
	let compose = std::fs::read_to_string(root.path().join("compose.yaml")).unwrap();
	assert!(
		compose.contains("      DATABASE_URL: sqlite:///data/analytics.sqlite3\n"),
		"{compose}"
	);
	assert!(
		compose.contains("      - analytics-data:/data\n"),
		"{compose}"
	);
	assert!(
		compose.contains("\nvolumes:\n  analytics-data:\n"),
		"{compose}"
	);
	let dockerfile = std::fs::read_to_string(root.path().join("Dockerfile")).unwrap();
	assert!(
		dockerfile.contains("mkdir -p /data /volumes/exports"),
		"{dockerfile}"
	);

	generate(root.path(), &config, Target::Kubernetes, false).expect("kubernetes");
	let kubernetes = std::fs::read_to_string(root.path().join("kubernetes.yaml")).unwrap();
	let analytics = kubernetes
		.split("\n---\n")
		.find(|manifest| manifest.contains("name: notes-analytics\nspec:\n  replicas:"))
		.expect("the analytics Deployment");
	assert!(
		analytics.contains("  replicas: 1\n  strategy:\n    type: Recreate\n"),
		"{analytics}"
	);
	assert!(
		analytics.contains(
			"            - name: DATABASE_URL\n              value: \
			 \"sqlite:///data/analytics.sqlite3\""
		),
		"{analytics}"
	);
	assert!(
		kubernetes.contains("  name: notes-analytics-data\n"),
		"{kubernetes}"
	);
}

/// Kubernetes names can't hold `_`, which Cargo package names often do, and
/// the kubelet can only check a numeric user isn't root.
#[test]
fn kubernetes_names_and_users_suit_the_kubelet() {
	let root = tempfile::tempdir().unwrap();
	project_with_services(root.path(), Db::Sqlite, "first-party");
	let toml = std::fs::read_to_string(root.path().join("leptoskit.toml"))
		.unwrap()
		.replace("name = \"notes\"", "name = \"my_notes\"");
	write(root.path(), "leptoskit.toml", &toml);
	let cargo = std::fs::read_to_string(root.path().join("Cargo.toml"))
		.unwrap()
		.replace("name = \"notes\"", "name = \"my_notes\"");
	write(root.path(), "Cargo.toml", &cargo);
	let config = LeptoskitConfig::from_toml(&toml, root.path()).expect("config");
	generate(root.path(), &config, Target::Kubernetes, false).expect("kubernetes");
	let kubernetes = std::fs::read_to_string(root.path().join("kubernetes.yaml")).unwrap();

	// Resource names, container names, labels, claims, and the Ingress's
	// backends; environment variables keep their `_`.
	for line in kubernetes.lines() {
		let name = [
			"  name: ",
			"        - name: ",
			"                name: ",
			"            claimName: ",
			"      app: ",
			"    app: ",
			"        app: ",
		]
		.into_iter()
		.find_map(|prefix| line.strip_prefix(prefix));

		if let Some(name) = name {
			assert!(!name.contains('_'), "{line}");
		}
	}

	assert!(
		kubernetes.contains("          image: my_notes\n"),
		"{kubernetes}"
	);
	assert!(
		kubernetes.contains("\"http://my-notes-analytics:3100\""),
		"{kubernetes}"
	);
	assert!(
		kubernetes.contains("\"http://my-notes:80\""),
		"{kubernetes}"
	);
	let pods = kubernetes
		.matches("      terminationGracePeriodSeconds: 30\n")
		.count();
	assert_eq!(
		kubernetes.matches("      securityContext:\n        runAsUser: 10001\n        runAsGroup: 10001\n        fsGroup: 10001\n").count(),
		pods,
		"{kubernetes}"
	);
}

#[test]
fn the_services_are_read_from_their_manifests() {
	let root = tempfile::tempdir().unwrap();
	let config = project_with_services(root.path(), Db::Postgres, "first-party");
	let project = Project::read(root.path(), &config).unwrap();
	let names: Vec<&str> = project
		.services
		.iter()
		.map(|service| service.name.as_str())
		.collect();
	assert_eq!(names, ["analytics", "billing", "cards"]);

	let analytics = &project.services[0];
	assert_eq!(
		analytics.build_path.as_deref(),
		Some("target/release/analytics_service")
	);
	assert_eq!(analytics.port, Some(3100));
	assert_eq!(
		analytics.public_prefixes,
		["/_leptoskit/services/analytics", "/_analytics"]
	);
	assert!(analytics.database);
	assert_eq!(analytics.pool, Some(4));

	let billing = &project.services[1];
	assert!(!billing.is_process());
	assert_eq!(
		billing.url.as_deref(),
		Some("https://billing.internal.example")
	);
}

/// Kubernetes with each role its own Deployment still sends the live paths
/// to the live Service once there's an Ingress for the services.
#[test]
fn split_roles_route_the_live_paths_beside_the_services() {
	let root = tempfile::tempdir().unwrap();
	let config = project_with_services(root.path(), Db::Postgres, "first-party");
	generate_with(root.path(), &config, Target::Kubernetes, false, true).unwrap();
	let manifest = std::fs::read_to_string(root.path().join("kubernetes.yaml")).unwrap();
	let ingress = &manifest[manifest.find("kind: Ingress").unwrap()..];
	let analytics = ingress
		.find("path: /_leptoskit/services/analytics")
		.unwrap();
	let live = ingress.find("path: /live").unwrap();
	let root_path = ingress.find("path: /\n").unwrap();
	assert!(analytics < live && live < root_path, "{ingress}");
	assert!(manifest.contains("name: notes-cards"));
}

#[test]
fn kubernetes_refuses_an_isolated_public_service() {
	let root = tempfile::tempdir().unwrap();
	let config = project_with_services(root.path(), Db::Postgres, "isolated");
	let error = generate(root.path(), &config, Target::Kubernetes, false)
		.unwrap_err()
		.to_string();
	assert!(
		error.contains("services.analytics is isolated and public") && error.contains("Cookie"),
		"{error}"
	);
}

#[test]
fn fly_and_systemd_refuse_services_they_cannot_run() {
	for target in [Target::Fly, Target::Systemd] {
		let root = tempfile::tempdir().unwrap();
		let config = project_with_services(root.path(), Db::Postgres, "first-party");
		let error = generate(root.path(), &config, target, false).unwrap_err();
		assert!(
			error.to_string().contains("services.analytics")
				&& error.to_string().contains("--target compose"),
			"{error}"
		);
	}
}

#[test]
fn the_connection_budget_counts_every_pool() {
	let root = tempfile::tempdir().unwrap();
	let config = project_with_services(root.path(), Db::Postgres, "first-party");
	let project = Project::read(root.path(), &config).unwrap();
	// The app: (1 host + 1 during a deploy) × (8 + 1). analytics: 2 × (4 +
	// 1). cards has no database. 5 kept back.
	let budget = super::services::Budget::new(8, 1, &project.services, 100);
	assert_eq!(budget.total, 18 + 10 + 5);
	assert!(budget.fits());
	let tight = super::services::Budget::new(8, 1, &project.services, 30);
	assert!(!tight.fits());
	assert!(tight.explain().contains("services.analytics"));
}
