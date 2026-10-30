//! The client against the real `/auth` routes, served in memory. The same
//! scenarios run with the browser's credentials, against a cookie jar that
//! behaves as a browser's does, and with a native app's, against an
//! in-memory secure store.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::header;
use chrono::TimeDelta;
use http_body_util::BodyExt;
use leptoskit_db::Db;
use leptoskit_query::BoxFuture;
use tower::ServiceExt;

use super::*;
use crate::Auth;
use crate::Keys;
use crate::Limit;
use crate::Limits;
use crate::Mailbox;
use crate::client::browser::Cookie;
use crate::client::native::SecretStore;
use crate::client::native::TokenKeeper;
use crate::client::protocol::Credentials;
use crate::client::protocol::Http;
use crate::client::protocol::HttpRequest;
use crate::client::protocol::HttpResponse;
use crate::client::protocol::Method;
use crate::client::protocol::Remote;
use crate::client::protocol::Unreachable;
use crate::client::protocol::error;

const ORIGIN: &str = "https://notes.example";
const ADA: &str = "ada@example.com";

struct Server {
	#[cfg_attr(
		not(feature = "ssr"),
		allow(dead_code, reason = "only server rendering adds routes")
	)]
	auth: Auth,
	router: Router,
	mailbox: Mailbox,
	db: Db,
}

impl Server {
	async fn new() -> Self {
		let db = leptoskit_db::testing::sqlite().await;
		let mailbox = Mailbox::new();
		let plenty = Limit::new(1000, Duration::from_secs(60));
		let auth = Auth::builder(&db, Keys::single("test", [9; 32]))
			.mailer(mailbox.clone())
			.origin(ORIGIN)
			.limits(Limits {
				code_per_address: plenty,
				code_per_ip: plenty,
				verify_per_ip: plenty,
			})
			.build()
			.unwrap();
		let router = Router::new().merge(auth.router()).layer(auth.layer());

		Self {
			auth,
			router,
			mailbox,
			db,
		}
	}

	fn code(&self) -> String {
		let mail = self.mailbox.latest_to(ADA).unwrap();
		mail.subject.rsplit(' ').next().unwrap().to_owned()
	}

	/// Makes every access token the server issued stop working, as time
	/// passing would.
	async fn expire_access_tokens(&self) {
		Session::update()
			.filter(|row| row.access_expires_at.is_not_null())
			.set(
				|row| row.access_expires_at,
				Some(leptoskit_db::now() - TimeDelta::seconds(1)),
			)
			.run(&self.db.system("test"))
			.await
			.unwrap();
	}

	fn browser(&self) -> Device {
		let http = InMemory::new(self, true);
		let jar = Arc::clone(&http.jar);
		Device {
			api: Arc::new(Remote::new(ClientConfig::new(), http.clone(), Cookie)),
			http,
			held: Box::new(move || jar.lock().unwrap().is_some()),
		}
	}

	fn native(&self) -> Native {
		self.native_with(Memory::default(), Clock::default())
	}

	fn native_with(&self, store: Memory, clock: Clock) -> Native {
		let http = InMemory::new(self, false);
		let now = clock.clone();
		let config = ClientConfig::new()
			.origin(format!("{ORIGIN}/"))
			.device_name("Ada's phone");
		Native {
			api: Arc::new(Remote::new(
				config,
				http.clone(),
				TokenKeeper::new(store, move || now.0.load(Ordering::SeqCst)),
			)),
			http,
			clock,
		}
	}
}

/// The routes, reached in memory. As a browser, it keeps the session cookie
/// in a jar, sends it back, and says which site the request is from; as a
/// native app, it does neither.
#[derive(Clone)]
struct InMemory {
	router: Router,
	browser: bool,
	jar: Arc<Mutex<Option<String>>>,
	sent: Arc<Mutex<Vec<String>>>,
	offline: Arc<AtomicBool>,
}

impl InMemory {
	fn new(server: &Server, browser: bool) -> Self {
		Self {
			router: server.router.clone(),
			browser,
			jar: Arc::default(),
			sent: Arc::default(),
			offline: Arc::default(),
		}
	}

	/// How many requests went to `path` since the last call.
	fn count(&self, path: &str) -> usize {
		let mut sent = self.sent.lock().unwrap();
		let count = sent.iter().filter(|url| url.ends_with(path)).count();
		sent.clear();
		count
	}
}

impl Http for InMemory {
	fn send(
		&self,
		request: HttpRequest,
	) -> BoxFuture<'static, Result<HttpResponse, ErrorResponse>> {
		let this = self.clone();
		Box::pin(async move {
			this.sent.lock().unwrap().push(request.url.clone());

			if this.offline.load(Ordering::SeqCst) {
				return Err(error("network", "offline"));
			}

			let mut builder = Request::builder()
				.method(request.method.as_str())
				.uri(&request.url);

			for (name, value) in request.headers() {
				builder = builder.header(name, value);
			}

			if this.browser {
				assert!(
					request.authorization.is_none(),
					"a browser's session stays in its cookie"
				);
				assert!(request.url.starts_with("/auth/"), "{}", request.url);
				builder = builder.header(header::ORIGIN, ORIGIN);

				if let Some(cookie) = this.jar.lock().unwrap().clone() {
					builder = builder.header(header::COOKIE, cookie);
				}
			} else {
				assert!(request.url.starts_with(ORIGIN), "{}", request.url);
			}

			let body = request.body.map_or_else(Body::empty, Body::from);
			let response = this
				.router
				.oneshot(builder.body(body).unwrap())
				.await
				.unwrap();

			if this.browser
				&& let Some(set_cookie) = response.headers().get(header::SET_COOKIE)
			{
				let set_cookie = set_cookie.to_str().unwrap();
				*this.jar.lock().unwrap() = (!set_cookie.contains("Max-Age=0"))
					.then(|| set_cookie.split(';').next().unwrap().to_owned());
			}

			let status = response.status().as_u16();
			let body = response.into_body().collect().await.unwrap().to_bytes();
			Ok(HttpResponse {
				status,
				body: body.to_vec(),
			})
		})
	}
}

/// A secure store in memory.
#[derive(Clone, Default)]
struct Memory(Arc<Mutex<Option<String>>>);

impl Memory {
	fn held(&self) -> Option<String> {
		self.0.lock().unwrap().clone()
	}
}

impl SecretStore for Memory {
	fn get(&self) -> BoxFuture<'_, Result<Option<String>, ErrorResponse>> {
		Box::pin(std::future::ready(Ok(self.held())))
	}

	fn set(&self, refresh_token: String) -> BoxFuture<'_, Result<(), ErrorResponse>> {
		*self.0.lock().unwrap() = Some(refresh_token);
		Box::pin(std::future::ready(Ok(())))
	}

	fn delete(&self) -> BoxFuture<'_, Result<(), ErrorResponse>> {
		*self.0.lock().unwrap() = None;
		Box::pin(std::future::ready(Ok(())))
	}
}

/// A clock the test moves, in milliseconds since the epoch.
#[derive(Clone)]
struct Clock(Arc<AtomicU64>);

impl Default for Clock {
	fn default() -> Self {
		Self(Arc::new(AtomicU64::new(1_800_000_000_000)))
	}
}

impl Clock {
	fn advance(&self, seconds: u64) {
		self.0.fetch_add(seconds * 1000, Ordering::SeqCst);
	}
}

/// A signed-in device, as every platform has one.
struct Device {
	api: Arc<dyn Api>,
	http: InMemory,
	/// Whether it holds credentials: the cookie, or a refresh token.
	held: Box<dyn Fn() -> bool>,
}

type NativeApi = Remote<InMemory, TokenKeeper<Memory>>;

struct Native {
	api: Arc<NativeApi>,
	http: InMemory,
	clock: Clock,
}

impl Native {
	fn store(&self) -> &Memory {
		self.api.credentials().store()
	}

	fn device(&self) -> Device {
		let store = self.store().clone();
		Device {
			api: Arc::clone(&self.api) as Arc<dyn Api>,
			http: self.http.clone(),
			held: Box::new(move || store.held().is_some()),
		}
	}
}

async fn sign_in(server: &Server, device: &Device) -> PublicSession {
	device.api.send_code(ADA.to_owned()).await.unwrap();
	device
		.api
		.verify_code(EmailCode {
			email: ADA.to_owned(),
			code: server.code(),
		})
		.await
		.unwrap()
}

async fn session_id(device: &Device) -> Option<Id<Session>> {
	device
		.api
		.session()
		.await
		.unwrap()
		.map(|session| session.id)
}

/// Everything a sign-in screen and a devices screen do, the same on every
/// platform. Returns the first device.
async fn signs_in_and_manages_devices(server: &Server, device: impl Fn() -> Device) -> Device {
	let laptop = device();
	let signed_in = signs_in(server, &laptop).await;
	lists_and_ends_devices(server, &laptop, signed_in.id, &device).await;
	signs_out_everywhere_and_here(server, &laptop, &device).await;
	laptop
}

async fn signs_in(server: &Server, laptop: &Device) -> PublicSession {
	assert_eq!(laptop.api.session().await.unwrap(), None);

	laptop.api.send_code(ADA.to_owned()).await.unwrap();
	let wrong = laptop
		.api
		.verify_code(EmailCode {
			email: ADA.to_owned(),
			code: "AAAA-AAAA".to_owned(),
		})
		.await
		.unwrap_err();
	assert_eq!(wrong.error, "invalid_code");
	assert!(!(laptop.held)());
	let invalid = laptop.api.send_code("nobody".to_owned()).await.unwrap_err();
	assert_eq!(invalid.error, "invalid_email");

	let signed_in = sign_in(server, laptop).await;
	assert!((laptop.held)());
	assert_eq!(session_id(laptop).await, Some(signed_in.id));
	signed_in
}

async fn lists_and_ends_devices(
	server: &Server,
	laptop: &Device,
	laptop_session: Id<Session>,
	device: impl Fn() -> Device,
) {
	let phone = device();
	sign_in(server, &phone).await;
	let tablet = device();
	let tablet_session = sign_in(server, &tablet).await;

	let devices = laptop.api.devices().await.unwrap();
	assert_eq!(devices.len(), 3);
	let current: Vec<_> = devices
		.iter()
		.filter(|device| device.current)
		.map(|device| device.session.id)
		.collect();
	assert_eq!(current, [laptop_session]);

	assert!(laptop.api.revoke(tablet_session.id).await.unwrap());
	assert!(!laptop.api.revoke(tablet_session.id).await.unwrap());
	assert_eq!(session_id(&tablet).await, None);

	laptop.api.revoke_others().await.unwrap();
	assert_eq!(session_id(&phone).await, None);
	assert_eq!(laptop.api.devices().await.unwrap().len(), 1);
}

async fn signs_out_everywhere_and_here(
	server: &Server,
	laptop: &Device,
	device: impl Fn() -> Device,
) {
	let desk = device();
	sign_in(server, &desk).await;
	desk.api.sign_out_everywhere().await.unwrap();
	assert!(!(desk.held)());
	assert_eq!(session_id(&desk).await, None);
	assert_eq!(session_id(laptop).await, None);
	let signed_out = laptop.api.devices().await.unwrap_err();
	assert_eq!(signed_out.error, "signed_out");
	assert_eq!(
		laptop.api.revoke_others().await.unwrap_err().error,
		"signed_out"
	);

	sign_in(server, laptop).await;
	assert!((laptop.held)());
	laptop.api.sign_out().await.unwrap();
	assert!(!(laptop.held)());
	assert_eq!(session_id(laptop).await, None);
	// Signing out again is harmless.
	laptop.api.sign_out().await.unwrap();
}

#[tokio::test]
async fn a_browser_signs_in_and_manages_devices_with_its_cookie() {
	let server = Server::new().await;
	let laptop = signs_in_and_manages_devices(&server, || server.browser()).await;
	// The cookie never needs renewing.
	assert_eq!(laptop.http.count("/auth/refresh"), 0);
}

#[tokio::test]
async fn a_native_app_signs_in_and_manages_devices_with_tokens() {
	let server = Server::new().await;
	let laptop = signs_in_and_manages_devices(&server, || server.native().device()).await;
	// Its session ended elsewhere, so it tried to renew, and was refused.
	assert!(laptop.http.count("/auth/refresh") > 0);
}

#[tokio::test]
async fn a_native_app_names_its_device_and_keeps_only_the_refresh_token() {
	let server = Server::new().await;
	let app = server.native();
	sign_in(&server, &app.device()).await;

	let devices = app.api.devices().await.unwrap();
	assert_eq!(devices[0].session.device, "Ada's phone");
	let refresh_token = app.store().held().unwrap();

	// Another run of the app finds its session from the stored token alone.
	let restarted = server.native_with(app.store().clone(), app.clock.clone());
	assert!(session_id(&restarted.device()).await.is_some());
	assert_eq!(restarted.http.count("/auth/refresh"), 1);
	assert_ne!(restarted.store().held().unwrap(), refresh_token);
}

#[tokio::test]
async fn a_native_app_renews_its_access_token_before_it_runs_out() {
	let server = Server::new().await;
	let app = server.native();
	sign_in(&server, &app.device()).await;
	let first = app.store().held();
	app.http.count("");

	// With more than a minute left of its 15, requests use the access token
	// as it is.
	app.clock.advance(13 * 60 + 20);
	app.api.devices().await.unwrap();
	assert_eq!(app.http.count("/auth/refresh"), 0);

	// Within the last minute, the client renews first.
	app.clock.advance(60);
	app.api.devices().await.unwrap();
	assert_eq!(app.http.count("/auth/refresh"), 1);
	assert_ne!(app.store().held(), first);
}

#[tokio::test]
async fn a_native_app_renews_after_the_server_refuses_its_access_token() {
	let server = Server::new().await;
	let app = server.native();
	sign_in(&server, &app.device()).await;
	app.http.count("");

	server.expire_access_tokens().await;
	assert_eq!(app.api.devices().await.unwrap().len(), 1);
	assert_eq!(app.http.count("/auth/refresh"), 1);

	// The session route answers a stale token with no session, not `401`.
	server.expire_access_tokens().await;
	assert!(session_id(&app.device()).await.is_some());
	assert_eq!(app.http.count("/auth/refresh"), 1);
}

#[tokio::test]
async fn concurrent_requests_share_one_renewal() {
	let server = Server::new().await;
	let app = server.native();
	sign_in(&server, &app.device()).await;
	app.http.count("");

	app.clock.advance(15 * 60);
	let (devices, session, revoked) = futures::join!(
		app.api.devices(),
		app.api.session(),
		app.api.revoke_others()
	);
	assert_eq!(devices.unwrap().len(), 1);
	assert!(session.unwrap().is_some());
	revoked.unwrap();
	assert_eq!(app.http.count("/auth/refresh"), 1);

	// The same when the server is the one to refuse the token.
	server.expire_access_tokens().await;
	let (first, second) = futures::join!(app.api.devices(), app.api.devices());
	first.unwrap();
	second.unwrap();
	assert_eq!(app.http.count("/auth/refresh"), 1);
}

#[tokio::test]
async fn a_failed_renewal_signs_a_native_app_out() {
	let server = Server::new().await;
	let app = server.native();
	sign_in(&server, &app.device()).await;
	let other = server.native();
	sign_in(&server, &other.device()).await;

	// Signing out everywhere ends this app's session too.
	other.api.sign_out_everywhere().await.unwrap();
	app.clock.advance(15 * 60);
	app.http.count("");
	assert_eq!(session_id(&app.device()).await, None);
	assert_eq!(app.http.count("/auth/refresh"), 1);
	assert_eq!(app.store().held(), None);

	// With nothing stored, there's nothing to renew.
	assert_eq!(session_id(&app.device()).await, None);
	assert_eq!(app.http.count("/auth/refresh"), 0);
}

#[tokio::test]
async fn an_unreachable_server_keeps_a_native_apps_refresh_token() {
	let server = Server::new().await;
	let app = server.native();
	sign_in(&server, &app.device()).await;
	let stored = app.store().held();

	app.clock.advance(15 * 60);
	app.http.offline.store(true, Ordering::SeqCst);
	assert_eq!(app.api.devices().await.unwrap_err().error, "network");
	assert_eq!(app.store().held(), stored);

	server.expire_access_tokens().await;
	app.http.offline.store(false, Ordering::SeqCst);
	app.api.devices().await.unwrap();

	// Signing out forgets the tokens even when the server can't hear it.
	app.http.offline.store(true, Ordering::SeqCst);
	assert_eq!(app.api.sign_out().await.unwrap_err().error, "network");
	assert_eq!(app.store().held(), None);
}

/// Answers every request with the same response.
struct Answer(u16, &'static str);

impl Http for Answer {
	fn send(
		&self,
		_request: HttpRequest,
	) -> BoxFuture<'static, Result<HttpResponse, ErrorResponse>> {
		Box::pin(std::future::ready(Ok(HttpResponse {
			status: self.0,
			body: self.1.as_bytes().to_vec(),
		})))
	}
}

#[tokio::test]
async fn odd_answers_become_errors() {
	let config = ClientConfig::new();
	let garbled = Remote::new(config.clone(), Answer(200, "<html>"), Cookie);
	assert_eq!(garbled.devices().await.unwrap_err().error, "server");

	let failed = Remote::new(config.clone(), Answer(502, "Bad Gateway"), Cookie);
	assert_eq!(failed.sign_out().await.unwrap_err().error, "server");
	assert_eq!(failed.revoke(Id::new()).await.unwrap_err().error, "server");

	let limited = Remote::new(
		config.clone(),
		Answer(
			429,
			r#"{"error":"rate_limited","message":"slow down","retry_after":30}"#,
		),
		Cookie,
	);
	let error = limited.send_code(ADA.to_owned()).await.unwrap_err();
	assert_eq!(
		(error.error.as_str(), error.retry_after),
		("rate_limited", Some(30))
	);

	// An out-of-date server signs a native app in without tokens.
	let keeper = TokenKeeper::new(Memory::default(), || 0);
	assert_eq!(keeper.keep(None).await.unwrap_err().error, "server");

	let input = EmailCode {
		email: ADA.to_owned(),
		code: "K7QM-R2XD".to_owned(),
	};
	assert!(!format!("{input:?}").contains("K7QM"));

	let unreachable = Remote::new(config, Unreachable, Cookie);
	assert_eq!(unreachable.session().await.unwrap_err().error, "config");
}

#[test]
fn requests_say_what_they_carry() {
	let request = HttpRequest {
		method: Method::Post,
		url: "/auth/email/send".to_owned(),
		authorization: None,
		body: Some("{}".to_owned()),
	};
	assert_eq!(
		request.headers(),
		[
			("accept", "application/json"),
			("content-type", "application/json")
		]
	);

	let request = HttpRequest {
		method: Method::Get,
		url: format!("{ORIGIN}/auth/session"),
		authorization: Some("Bearer abc".to_owned()),
		body: None,
	};
	assert_eq!(
		request.headers(),
		[
			("accept", "application/json"),
			("authorization", "Bearer abc")
		]
	);
	assert_eq!(request.method.as_str(), "GET");

	let config = ClientConfig::new().origin(ORIGIN).device_name("Desk");
	assert_eq!(config.origin.as_deref(), Some(ORIGIN));
	assert_eq!(config.device_name.as_deref(), Some("Desk"));
}

/// The scopes run on whichever client is configured; this one test owns the
/// process-wide client.
#[tokio::test]
async fn the_scopes_call_the_configured_client() {
	use leptos::prelude::Owner;
	use leptoskit_query::NetworkMode;
	use leptoskit_query::QueryClient;
	use leptoskit_query::QueryDefaults;
	use leptoskit_query::RefetchOn;
	use leptoskit_query::use_mutation_in;

	let _ = any_spawner::Executor::init_tokio();
	let owner = Owner::new();
	owner.set();

	// Before it's configured, and configured without a way to reach the
	// server, the client says what's missing.
	let unconfigured = if cfg!(feature = "ssr") {
		"server"
	} else {
		"config"
	};
	assert_eq!(
		api().send_code(ADA.to_owned()).await.unwrap_err().error,
		unconfigured
	);
	configure(ClientConfig::new());
	assert_eq!(api().sign_out().await.unwrap_err().error, unconfigured);

	let server = Server::new().await;
	let browser = server.browser();
	install(Arc::clone(&browser.api));
	let client = QueryClient::new().with_defaults(
		QueryDefaults::new()
			.with_refetch_on_window_focus(RefetchOn::Never)
			.with_refetch_on_reconnect(RefetchOn::Never)
			.with_network_mode(NetworkMode::Always),
	);
	client.provide();

	assert_eq!(use_auth().await, Ok(None));
	use_mutation_in(client.clone(), send_code())
		.mutate_async(ADA.to_owned())
		.await
		.unwrap();
	let signed_in = use_mutation_in(client.clone(), verify_code())
		.mutate_async(EmailCode {
			email: ADA.to_owned(),
			code: server.code(),
		})
		.await
		.unwrap();
	assert_eq!(
		client.get_query_data(&session(), &()),
		Some(Some(signed_in.clone()))
	);

	let phone = server.browser();
	let phone_session = sign_in(&server, &phone).await;
	sign_in(&server, &server.browser()).await;
	assert_eq!(use_devices().await.unwrap().len(), 3);
	assert!(
		use_mutation_in(client.clone(), revoke_device())
			.mutate_async(phone_session.id)
			.await
			.unwrap()
	);
	assert_eq!(client.fetch_query(&devices(), ()).await.unwrap().len(), 2);
	use_mutation_in(client.clone(), revoke_other_devices())
		.mutate_async(())
		.await
		.unwrap();
	assert_eq!(client.fetch_query(&devices(), ()).await.unwrap().len(), 1);

	use_mutation_in(client.clone(), sign_out_everywhere_mutation())
		.mutate_async(())
		.await
		.unwrap();
	assert_eq!(client.get_query_data(&session(), &()), Some(None));
	assert_eq!(client.get_query_data(&devices(), &()), Some(Vec::new()));

	sign_in(&server, &browser).await;
	client.set_query_data(&session(), (), Some(signed_in));
	use_mutation_in(client.clone(), sign_out_mutation())
		.mutate_async(())
		.await
		.unwrap();
	assert_eq!(client.get_query_data(&session(), &()), Some(None));
	assert_eq!(session_id(&browser).await, None);
}

/// While the server renders, the client reads the request's own session.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn server_rendering_reads_the_requests_session() {
	use axum::Json;
	use axum::routing::get;
	use leptos::prelude::Owner;
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
					let session = Rendering.session().await;
					let devices = Rendering.devices().await;
					(session, devices.map(|devices| devices.len()))
				})
			});
			Json(reads.await)
		}
	});
	let browser = server.browser();
	let rendering = Router::new()
		.route("/auth/render", render)
		.merge(server.auth.router())
		.layer(server.auth.layer());
	let page = || {
		let http = InMemory {
			router: rendering.clone(),
			..browser.http.clone()
		};
		async move {
			let response = http
				.send(HttpRequest {
					method: Method::Get,
					url: "/auth/render".to_owned(),
					authorization: None,
					body: None,
				})
				.await
				.unwrap();
			serde_json::from_slice::<(
				Result<Option<PublicSession>, ErrorResponse>,
				Result<usize, ErrorResponse>,
			)>(&response.body)
			.unwrap()
		}
	};

	let (session, devices) = page().await;
	assert_eq!(session, Ok(None));
	assert_eq!(devices.unwrap_err().error, "signed_out");

	let signed_in = sign_in(&server, &browser).await;
	let (session, devices) = page().await;
	assert_eq!(
		session.unwrap().map(|session| session.id),
		Some(signed_in.id)
	);
	assert_eq!(devices, Ok(1));

	// Changes run in the browser, not while the server renders.
	assert_eq!(Rendering.sign_out().await.unwrap_err().error, "server");
	for refused in [
		Rendering.send_code(ADA.to_owned()).await,
		Rendering.revoke_others().await,
		Rendering.sign_out_everywhere().await,
		Rendering.revoke(Id::new()).await.map(|_| ()),
		Rendering
			.verify_code(EmailCode::default())
			.await
			.map(|_| ()),
	] {
		assert_eq!(refused.unwrap_err().error, "server");
	}
}
