//! The client against the real routes and layers, served in memory: every
//! scope runs against a real `QueryClient`, through a sender that keeps a
//! browser's cookie jar, and the server-rendering reads run inside the
//! layers under a leptos owner.

use std::sync::Arc;
use std::sync::Mutex;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::header;
use axum::routing::get;
use http_body_util::BodyExt;
use leptos::prelude::Owner;
use leptoskit_auth::Auth;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::client::HttpResponse;
use leptoskit_auth::client::Method;
use leptoskit_query::BoxFuture;
use leptoskit_query::NetworkMode;
use leptoskit_query::QueryClient;
use leptoskit_query::QueryDefaults;
use leptoskit_query::RefetchOn;
use leptoskit_query::use_mutation_in;
use tower::ServiceExt;

use super::*;
use crate::ClientAuth;
use crate::GrantType;
use crate::Idp;
use crate::NewClientSpec;
use crate::Portal;
use crate::SigningAlgorithm;

const ORIGIN: &str = "https://notes.example";
const CALLBACK: &str = "https://partner.example/callback";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

/// The server the sender reaches, and the browser's cookie.
struct Browser {
	router: Router,
	cookie: Option<String>,
}

static BROWSER: Mutex<Option<Browser>> = Mutex::new(None);

/// Sends as a browser does: same origin, with the cookie it holds.
fn sender(
	method: Method,
	path: String,
	body: Option<String>,
) -> BoxFuture<'static, Result<HttpResponse, ErrorResponse>> {
	let (router, cookie) = {
		let browser = BROWSER.lock().unwrap();
		let browser = browser.as_ref().expect("a browser is set up");
		(browser.router.clone(), browser.cookie.clone())
	};
	Box::pin(async move {
		let mut builder = Request::builder()
			.method(method.as_str())
			.uri(&path)
			.header(header::ORIGIN, ORIGIN);
		if body.is_some() {
			builder = builder.header(header::CONTENT_TYPE, "application/json");
		}
		if let Some(cookie) = cookie {
			builder = builder.header(header::COOKIE, cookie);
		}
		let response = router
			.oneshot(
				builder
					.body(body.map_or_else(Body::empty, Body::from))
					.unwrap(),
			)
			.await
			.unwrap();
		let status = response.status().as_u16();
		let body = response.into_body().collect().await.unwrap().to_bytes();
		Ok(HttpResponse {
			status,
			body: body.to_vec(),
		})
	})
}

struct Server {
	auth: Auth,
	idp: Idp,
	router: Router,
	mailbox: Mailbox,
}

impl Server {
	async fn new() -> Self {
		let db = leptoskit_db::testing::sqlite().await;
		let mailbox = Mailbox::new();
		let auth = Auth::builder(&db, Keys::single("test", [6; 32]))
			.origin(ORIGIN)
			.mailer(mailbox.clone())
			.build()
			.unwrap();
		let idp = Idp::builder(&db, &auth)
			.developer_portal(Portal::default())
			.build()
			.unwrap();
		let router = Router::new()
			.merge(auth.router())
			.merge(idp.router())
			.layer(auth.layer())
			.layer(idp.layer());
		Self {
			auth,
			idp,
			router,
			mailbox,
		}
	}

	async fn send(&self, request: Request<Body>) -> axum::response::Response {
		self.router.clone().oneshot(request).await.unwrap()
	}

	/// Signs a browser in, returning its cookie.
	async fn sign_in(&self, address: &str) -> String {
		let post = |path: &str, body: String| {
			Request::post(path)
				.header(header::ORIGIN, ORIGIN)
				.header(header::CONTENT_TYPE, "application/json")
				.body(Body::from(body))
				.unwrap()
		};
		self.send(post(
			"/auth/email/send",
			serde_json::json!({ "email": address }).to_string(),
		))
		.await;
		let code = self
			.mailbox
			.latest_to(address)
			.unwrap()
			.subject
			.rsplit(' ')
			.next()
			.unwrap()
			.to_owned();
		let verified = self
			.send(post(
				"/auth/email/verify",
				serde_json::json!({ "email": address, "code": code }).to_string(),
			))
			.await;
		let cookie = verified.headers()[header::SET_COOKIE].to_str().unwrap();
		cookie.split(';').next().unwrap().to_owned()
	}

	async fn register(&self, grant_types: Vec<GrantType>) -> String {
		let (client, _) = self
			.idp
			.register_client(NewClientSpec {
				name: "Partner".into(),
				redirect_uris: vec![CALLBACK.into()],
				post_logout_redirect_uris: Vec::new(),
				scopes: vec!["openid".into(), "profile".into()],
				auth_method: ClientAuth::None,
				grant_types,
				jwks: None,
				id_token_alg: SigningAlgorithm::Es256,
				first_party: false,
				introspection: false,
			})
			.await
			.unwrap();
		client.client_id
	}

	/// Starts a sign-in as `cookie`'s browser, returning the consent handle.
	async fn consent_handle(&self, client_id: &str, cookie: &str) -> String {
		let query = url::form_urlencoded::Serializer::new(String::new())
			.extend_pairs([
				("response_type", "code"),
				("client_id", client_id),
				("redirect_uri", CALLBACK),
				("scope", "openid profile"),
				("code_challenge", CHALLENGE),
				("code_challenge_method", "S256"),
			])
			.finish();
		let response = self
			.send(
				Request::get(format!("/oauth/authorize?{query}"))
					.header(header::COOKIE, cookie)
					.body(Body::empty())
					.unwrap(),
			)
			.await;
		let to = response.headers()[header::LOCATION].to_str().unwrap();
		let to = url::Url::parse(ORIGIN).unwrap().join(to).unwrap();
		to.query_pairs()
			.find(|(name, _)| name == "request")
			.unwrap()
			.1
			.into_owned()
	}

	fn browse(&self, cookie: Option<String>) {
		*BROWSER.lock().unwrap() = Some(Browser {
			router: self.router.clone(),
			cookie,
		});
	}
}

fn query_client() -> QueryClient {
	let client = QueryClient::new().with_defaults(
		QueryDefaults::new()
			.with_refetch_on_window_focus(RefetchOn::Never)
			.with_refetch_on_reconnect(RefetchOn::Never)
			.with_network_mode(NetworkMode::Always),
	);
	client.provide();
	client
}

#[tokio::test]
async fn the_scopes_drive_every_screen_over_the_routes() {
	let _ = any_spawner::Executor::init_tokio();
	let owner = Owner::new();
	owner.set();
	let server = Server::new().await;
	install(Arc::new(remote::Remote::new(sender)));
	let client = query_client();

	// Signed out, every screen hears `login_required`.
	server.browse(None);
	assert_eq!(
		client
			.fetch_query(&connected_apps(), ())
			.await
			.unwrap_err()
			.error,
		"login_required"
	);

	let cookie = server.sign_in("ada@example.com").await;
	server.browse(Some(cookie.clone()));

	// Consent: read the request, approve it, and the app is connected.
	let client_id = server.register(vec![GrantType::AuthorizationCode]).await;
	let handle = server.consent_handle(&client_id, &cookie).await;
	let request = client
		.fetch_query(&consent(), handle.clone())
		.await
		.unwrap();
	assert_eq!(request.app.name, "Partner");
	assert_eq!(request.redirect_host, "partner.example");
	assert_eq!(
		client
			.fetch_query(&consent(), "unknown".into())
			.await
			.unwrap_err()
			.error,
		"not_found"
	);
	let redirect = use_mutation_in(client.clone(), decide_consent())
		.mutate_async(ConsentDecision {
			request: handle,
			approve: true,
		})
		.await
		.unwrap();
	assert!(redirect.redirect_to.starts_with(CALLBACK), "{redirect:?}");
	assert_eq!(
		client
			.fetch_query(&connected_apps(), ())
			.await
			.unwrap()
			.len(),
		1
	);

	// Disconnecting it empties the list; a second time finds nothing.
	let disconnect = use_mutation_in(client.clone(), disconnect_app());
	assert!(disconnect.mutate_async(client_id.clone()).await.unwrap());
	assert_eq!(
		client.fetch_query(&connected_apps(), ()).await.unwrap(),
		[] as [ConnectedApp; 0]
	);
	assert!(!disconnect.mutate_async(client_id).await.unwrap());

	// A device: read the code someone typed, then approve it.
	let device_app = server.register(vec![GrantType::DeviceCode]).await;
	let started = server
		.send(
			Request::post("/oauth/device_authorization")
				.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
				.body(Body::from(format!("client_id={device_app}&scope=openid")))
				.unwrap(),
		)
		.await;
	let started: crate::DeviceAuthorization =
		serde_json::from_slice(&started.into_body().collect().await.unwrap().to_bytes()).unwrap();
	let typed = started.user_code.to_lowercase();
	let request = client
		.fetch_query(&device_request(), typed.clone())
		.await
		.unwrap();
	assert_eq!(request.user_code, started.user_code);
	use_mutation_in(client.clone(), decide_device())
		.mutate_async(DeviceDecision {
			user_code: typed,
			approve: true,
		})
		.await
		.unwrap();
	assert_eq!(
		client
			.fetch_query(&connected_apps(), ())
			.await
			.unwrap()
			.len(),
		1
	);

	// The developer portal: register, change, rotate, and delete an app.
	let registered = use_mutation_in(client.clone(), register_app())
		.mutate_async(NewApp {
			name: "Ada's app".into(),
			redirect_uris: vec![CALLBACK.into()],
			post_logout_redirect_uris: Vec::new(),
			scopes: vec!["openid".into()],
			auth_method: ClientAuth::SecretBasic,
			grant_types: crate::default_grant_types(),
			jwks: None,
		})
		.await
		.unwrap();
	assert!(registered.secret.is_some());
	let mine = client.fetch_query(&my_apps(), ()).await.unwrap();
	assert_eq!(mine.len(), 1);
	let changed = use_mutation_in(client.clone(), update_app())
		.mutate_async(AppChanges {
			client_id: registered.app.client_id.clone(),
			name: "Ada's notes".into(),
			redirect_uris: vec![CALLBACK.into()],
			post_logout_redirect_uris: Vec::new(),
			scopes: vec!["openid".into(), "profile".into()],
			jwks: None,
		})
		.await
		.unwrap();
	assert_eq!(changed.name, "Ada's notes");
	let rotated = use_mutation_in(client.clone(), rotate_app_secret())
		.mutate_async(registered.app.client_id.clone())
		.await
		.unwrap();
	assert_ne!(rotated.secret, registered.secret);
	let refused = use_mutation_in(client.clone(), register_app())
		.mutate_async(NewApp {
			name: String::new(),
			redirect_uris: Vec::new(),
			post_logout_redirect_uris: Vec::new(),
			scopes: Vec::new(),
			auth_method: ClientAuth::None,
			grant_types: crate::default_grant_types(),
			jwks: None,
		})
		.await
		.unwrap_err();
	assert_eq!(refused.error, "invalid_client_metadata");
	let delete = use_mutation_in(client.clone(), delete_app());
	assert!(
		delete
			.mutate_async(registered.app.client_id.clone())
			.await
			.unwrap()
	);
	assert!(!delete.mutate_async(registered.app.client_id).await.unwrap());
	assert_eq!(
		client.fetch_query(&my_apps(), ()).await.unwrap(),
		[] as [ClientInfo; 0]
	);
}

/// While the server renders, the reads come from the request's own session.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn server_rendering_reads_the_requests_session() {
	use leptos::prelude::provide_context;
	use leptos::reactive::computed::ScopedFuture;

	use super::server::Server as Rendering;

	let server = Server::new().await;
	let render = get(|request: axum::extract::Request| {
		async move {
			let (parts, _) = request.into_parts();
			let owner = Owner::new();
			let reads = owner.with(|| {
				provide_context(parts);
				ScopedFuture::new(async {
					let apps = Rendering.connected_apps().await.map(|apps| apps.len());
					let mine = Rendering.my_apps().await.map(|apps| apps.len());
					let consent = Rendering.consent("unknown".into()).await.map(|_| ());
					let device = Rendering.device("BCDF-GHJK".into()).await.map(|_| ());
					(apps, mine, consent, device)
				})
			});
			Json(reads.await)
		}
	});
	let rendering = Router::new()
		.route("/render", render)
		.merge(server.auth.router())
		.layer(server.auth.layer())
		.layer(server.idp.layer());
	let page = |cookie: Option<&str>| {
		let mut request = Request::get("/render");
		if let Some(cookie) = cookie {
			request = request.header(header::COOKIE, cookie);
		}
		let request = request.body(Body::empty()).unwrap();
		let rendering = rendering.clone();
		async move {
			let response = rendering.oneshot(request).await.unwrap();
			let body = response.into_body().collect().await.unwrap().to_bytes();
			serde_json::from_slice::<(
				Result<usize, ErrorResponse>,
				Result<usize, ErrorResponse>,
				Result<(), ErrorResponse>,
				Result<(), ErrorResponse>,
			)>(&body)
			.unwrap()
		}
	};

	let (apps, ..) = page(None).await;
	assert_eq!(apps.unwrap_err().error, "login_required");

	let cookie = server.sign_in("ada@example.com").await;
	let (apps, mine, consent, device) = page(Some(&cookie)).await;
	assert_eq!(apps, Ok(0));
	assert_eq!(mine, Ok(0));
	assert_eq!(consent.unwrap_err().error, "not_found");
	assert_eq!(device.unwrap_err().error, "not_found");

	// Changes run in the browser, not while the server renders.
	for refused in [
		Rendering
			.decide_consent(ConsentDecision {
				request: String::new(),
				approve: true,
			})
			.await
			.map(|_| ()),
		Rendering
			.decide_device(DeviceDecision {
				user_code: String::new(),
				approve: true,
			})
			.await,
		Rendering.disconnect(String::new()).await.map(|_| ()),
		Rendering.rotate_app_secret(String::new()).await.map(|_| ()),
		Rendering.delete_app(String::new()).await.map(|_| ()),
	] {
		assert_eq!(refused.unwrap_err().error, "server");
	}

	// Outside a request, the reads say what's missing.
	assert_eq!(
		Rendering.connected_apps().await.unwrap_err().error,
		"config"
	);
}

#[test]
fn only_failures_that_may_pass_are_retried() {
	let retry = transient();
	let failure = |code: &str| remote::error(code, "");
	assert!(retry.should_retry(0, &failure("network")));
	assert!(retry.should_retry(2, &failure("server")));
	assert!(!retry.should_retry(3, &failure("network")));
	assert!(!retry.should_retry(0, &failure("not_found")));
	assert!(!retry.should_retry(0, &failure("login_required")));
}
