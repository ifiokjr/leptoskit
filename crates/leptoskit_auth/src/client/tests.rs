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
			.solana(crate::SolanaOptions {
				sign_in: true,
				..crate::SolanaOptions::default()
			})
			.limits(Limits {
				code_per_address: plenty,
				code_per_ip: plenty,
				verify_per_ip: plenty,
				second_factor_per_account: plenty,
				ceremony_per_ip: plenty,
				password_per_address: plenty,
				..Limits::default()
			});
		#[cfg(feature = "password")]
		let auth = auth.passwords(crate::PasswordOptions::default());
		let auth = auth.build().unwrap();
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
			held: Box::new(move || jar.lock().unwrap().contains_key("__Host-session")),
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

/// The routes, reached in memory. As a browser, it keeps cookies in a jar
/// by name, sends them back, and says which site the request is from; as a
/// native app, it does neither.
#[derive(Clone)]
struct InMemory {
	router: Router,
	browser: bool,
	jar: Arc<Mutex<std::collections::BTreeMap<String, String>>>,
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

				let cookies = this
					.jar
					.lock()
					.unwrap()
					.iter()
					.map(|(name, value)| format!("{name}={value}"))
					.collect::<Vec<_>>()
					.join("; ");

				if !cookies.is_empty() {
					builder = builder.header(header::COOKIE, cookies);
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

			if this.browser {
				let mut jar = this.jar.lock().unwrap();

				for set_cookie in response.headers().get_all(header::SET_COOKIE) {
					let set_cookie = set_cookie.to_str().unwrap();
					let (name, value) = set_cookie
						.split(';')
						.next()
						.unwrap()
						.split_once('=')
						.unwrap();

					if set_cookie.contains("Max-Age=0") {
						jar.remove(name);
					} else {
						jar.insert(name.to_owned(), value.to_owned());
					}
				}
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
	let step = device
		.api
		.verify_code(EmailCode {
			email: ADA.to_owned(),
			code: server.code(),
		})
		.await
		.unwrap();
	let SignInStep::SignedIn(session) = step else {
		panic!("no second factor yet: {step:?}");
	};
	session
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

/// What an authenticator app computes.
#[path = "../../tests/support/totp.rs"]
mod authenticator;

/// Turns on two-step verification from one device, finishes signing in on
/// another with a recovery code, steps up, and turns it off again.
async fn uses_two_steps(server: &Server, device: impl Fn() -> Device) {
	let laptop = device();
	let codes = turns_on(server, &laptop).await;
	let phone = device();
	signs_in_with_a_recovery_code(server, &phone, &laptop, &codes).await;
	steps_up_and_turns_off(&phone, &codes).await;
}

/// Signs in on `laptop` and turns an authenticator on, returning the
/// recovery codes.
async fn turns_on(server: &Server, laptop: &Device) -> Vec<String> {
	sign_in(server, laptop).await;
	assert_eq!(laptop.api.mfa().await.unwrap(), MfaStatus::default());

	let setup = laptop.api.begin_totp().await.unwrap();
	let wrong = laptop.api.confirm_totp("000000".into()).await.unwrap_err();
	assert_eq!(wrong.error, "invalid_code");
	let confirmed = laptop
		.api
		.confirm_totp(authenticator::totp(&setup.secret, 0))
		.await
		.unwrap();
	assert_eq!(confirmed.session.aal, crate::Aal::Two);
	// The session's token was replaced, and the device kept the new one.
	assert_eq!(session_id(laptop).await, Some(confirmed.session.id));
	assert_eq!(
		laptop.api.mfa().await.unwrap(),
		MfaStatus {
			passkeys: 0,
			required: false,
			totp: true,
			recovery_codes: 10,
			password: false,
		}
	);
	confirmed.recovery_codes.unwrap().codes
}

/// The emailed code on `phone` asks for the second step, which a recovery
/// code finishes, signing `laptop` out.
async fn signs_in_with_a_recovery_code(
	server: &Server,
	phone: &Device,
	laptop: &Device,
	codes: &[String],
) {
	phone.api.send_code(ADA.to_owned()).await.unwrap();
	let step = phone
		.api
		.verify_code(EmailCode {
			email: ADA.to_owned(),
			code: server.code(),
		})
		.await
		.unwrap();
	let SignInStep::SecondFactor(prompt) = step else {
		panic!("the account has a second factor: {step:?}");
	};
	assert_eq!(prompt.factors, [Factor::Totp, Factor::RecoveryCode]);
	assert!(prompt.expires_in > 240);
	assert!(!(phone.held)(), "nothing is signed in yet");
	let wrong = phone
		.api
		.complete_sign_in("000000".into())
		.await
		.unwrap_err();

	assert_eq!(wrong.error, "invalid_code");
	let signed_in = phone.api.complete_sign_in(codes[0].clone()).await.unwrap();
	assert!((phone.held)());
	assert_eq!(session_id(phone).await, Some(signed_in.id));
	// A recovery code signs every other device out.
	assert_eq!(session_id(laptop).await, None);
	let again = phone
		.api
		.complete_sign_in(codes[1].clone())
		.await
		.unwrap_err();
	assert_eq!(again.error, "sign_in_expired");
}

async fn steps_up_and_turns_off(phone: &Device, codes: &[String]) {
	let stepped_up = phone.api.step_up(codes[1].clone()).await.unwrap();
	assert!(stepped_up.mfa_at.is_some());
	assert_eq!(session_id(phone).await, Some(stepped_up.id));
	let fresh = phone.api.regenerate_recovery_codes().await.unwrap();
	assert_eq!(fresh.codes.len(), 10);
	phone.api.disable_totp().await.unwrap();
	assert_eq!(phone.api.mfa().await.unwrap(), MfaStatus::default());
	assert_eq!(
		phone.api.disable_totp().await.unwrap_err().error,
		"not_enrolled"
	);
}

#[tokio::test]
async fn a_browser_uses_two_step_verification_with_its_cookies() {
	let server = Server::new().await;
	uses_two_steps(&server, || server.browser()).await;
}

#[tokio::test]
async fn a_native_app_uses_two_step_verification_with_tokens() {
	let server = Server::new().await;
	uses_two_steps(&server, || server.native().device()).await;
}

#[tokio::test]
async fn a_native_app_forgets_the_second_step_once_it_is_over() {
	let server = Server::new().await;
	let laptop = server.native();
	sign_in(&server, &laptop.device()).await;
	let setup = laptop.api.begin_totp().await.unwrap();
	laptop
		.api
		.confirm_totp(authenticator::totp(&setup.secret, 0))
		.await
		.unwrap();

	let phone = server.native();
	phone.api.send_code(ADA.to_owned()).await.unwrap();
	phone
		.api
		.verify_code(EmailCode {
			email: ADA.to_owned(),
			code: server.code(),
		})
		.await
		.unwrap();
	assert!(phone.api.credentials().second_factor().is_some());
	// Signing out abandons it.
	phone.api.sign_out().await.unwrap();
	assert_eq!(phone.api.credentials().second_factor(), None);
	assert_eq!(
		phone
			.api
			.complete_sign_in("000000".into())
			.await
			.unwrap_err()
			.error,
		"sign_in_expired"
	);
}

#[cfg(feature = "password")]
const GOOD_PASSWORD: &str = "correct horse battery staple";

/// Setting a password, signing in with it on another device, and resetting
/// it from a third, which signs the others out: the same on every
/// platform.
#[cfg(feature = "password")]
async fn uses_a_password(server: &Server, device: impl Fn() -> Device) {
	let laptop = sets_a_password(server, &device).await;
	let phone = signs_in_with_a_password(&device).await;
	let tablet = device();
	tablet.api.send_code(ADA.to_owned()).await.unwrap();
	let reset = tablet
		.api
		.reset_password(PasswordReset {
			email: ADA.to_owned(),
			code: server.code(),
			password: "a brand new and longer phrase".to_owned(),
		})
		.await
		.unwrap();
	assert!(matches!(reset, SignInStep::SignedIn(_)), "{reset:?}");
	assert_eq!(session_id(&phone).await, None, "the reset signed it out");
	assert_eq!(session_id(&laptop).await, None);
	tablet.api.remove_password().await.unwrap();
	assert!(!tablet.api.mfa().await.unwrap().password);
}

#[cfg(feature = "password")]
async fn sets_a_password(server: &Server, device: &impl Fn() -> Device) -> Device {
	let laptop = device();
	sign_in(server, &laptop).await;
	let settings = laptop.api.password_settings().await.unwrap();
	assert!(settings.enabled);
	assert_eq!(settings.min_length, 15);
	let weak = laptop
		.api
		.set_password("short".to_owned())
		.await
		.unwrap_err();
	assert_eq!(weak.error, "weak_password");
	assert_eq!(weak.message, "use at least 15 characters");
	laptop
		.api
		.set_password(GOOD_PASSWORD.to_owned())
		.await
		.unwrap();
	assert!(laptop.api.mfa().await.unwrap().password);
	laptop
}

#[cfg(feature = "password")]
async fn signs_in_with_a_password(device: &impl Fn() -> Device) -> Device {
	let phone = device();
	let wrong = phone
		.api
		.sign_in_with_password(PasswordInput {
			email: ADA.to_owned(),
			password: "a wrong and long password".to_owned(),
		})
		.await
		.unwrap_err();
	assert_eq!(wrong.error, "invalid_password");
	assert!(!(phone.held)());
	let input = PasswordInput {
		email: ADA.to_owned(),
		password: GOOD_PASSWORD.to_owned(),
	};
	assert!(!format!("{input:?}").contains(GOOD_PASSWORD));
	let step = phone.api.sign_in_with_password(input).await.unwrap();
	assert!(matches!(step, SignInStep::SignedIn(_)), "{step:?}");
	assert!((phone.held)());
	phone
}

#[cfg(feature = "password")]
#[tokio::test]
async fn a_browser_uses_a_password_with_its_cookie() {
	let server = Server::new().await;
	uses_a_password(&server, || server.browser()).await;
}

#[cfg(feature = "password")]
#[tokio::test]
async fn a_native_app_uses_a_password_with_tokens() {
	let server = Server::new().await;
	uses_a_password(&server, || server.native().device()).await;
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

	// So do other crates' requests.
	server.expire_access_tokens().await;
	let sent = app
		.api
		.send(Method::Post, "/auth/sessions/revoke-others".into(), None)
		.await
		.unwrap();
	assert_eq!(sent.status, 204);
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
/// The scopes share one installed client, so the tests that install one
/// take turns.
static INSTALLED: futures::lock::Mutex<()> = futures::lock::Mutex::new(());

#[tokio::test]
async fn the_scopes_call_the_configured_client() {
	let _turn = INSTALLED.lock().await;
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
	assert_eq!(
		send(Method::Get, "/auth/sessions", None)
			.await
			.unwrap_err()
			.error,
		unconfigured
	);

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
	let SignInStep::SignedIn(signed_in) = signed_in else {
		panic!("no second factor yet");
	};
	assert_eq!(
		client.get_query_data(&session(), &()),
		Some(Some(signed_in.clone()))
	);

	// Other crates' routes go out the same way, with the session.
	let sessions = send(Method::Get, "/auth/sessions", None).await.unwrap();
	assert_eq!(sessions.status, 200);
	assert_eq!(
		serde_json::from_slice::<Vec<crate::Device>>(&sessions.body)
			.unwrap()
			.len(),
		1
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

/// How many devices, plus what the other reads counted, or the first
/// refusal.
fn total(
	devices: Result<Vec<crate::Device>, ErrorResponse>,
	others: [Result<usize, ErrorResponse>; 7],
) -> Result<usize, ErrorResponse> {
	let mut count = devices?.len();

	for other in others {
		count += other?;
	}

	Ok(count)
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
					// The other reads answer alike, signed in or not.
					let others = [
						Rendering.passkeys().await.map(|list| list.len()),
						Rendering.providers().await.map(|list| list.len()),
						Rendering.identities().await.map(|list| list.len()),
						Rendering
							.merge_offer()
							.await
							.map(|offer| usize::from(offer.is_some())),
						Rendering
							.mfa()
							.await
							.map(|status| usize::from(status.enrolled())),
						Rendering.wallets().await.map(|list| list.len()),
						Rendering
							.wallet_settings()
							.await
							.map(|settings| usize::from(!settings.enabled)),
					];
					let devices = total(devices, others);
					(session, devices)
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
		Rendering
			.complete_sign_in("123456".into())
			.await
			.map(|_| ()),
		Rendering.step_up("123456".into()).await.map(|_| ()),
		Rendering.begin_totp().await.map(|_| ()),
		Rendering.confirm_totp("123456".into()).await.map(|_| ()),
		Rendering.disable_totp().await,
		Rendering.regenerate_recovery_codes().await.map(|_| ()),
	] {
		assert_eq!(refused.unwrap_err().error, "server");
	}

	for refused in [
		Rendering
			.passkey_sign_in(PasskeyMode::Prompt)
			.await
			.map(|_| ()),
		Rendering.add_passkey(None).await.map(|_| ()),
		Rendering.complete_sign_in_with_passkey().await.map(|_| ()),
		Rendering.step_up_with_passkey().await.map(|_| ()),
		Rendering
			.rename_passkey(RenamePasskey {
				id: Id::new(),
				name: "x".into(),
			})
			.await,
		Rendering.remove_passkey(Id::new()).await,
		Rendering
			.start_provider(
				StartProvider {
					provider: "google".into(),
					redirect_to: None,
				},
				false,
			)
			.await,
		Rendering.continue_provider_sign_up().await.map(|_| ()),
		Rendering.unlink_identity(Id::new()).await,
		Rendering.merge_accounts(MergeInput::default()).await,
		Rendering.cancel_merge().await,
	] {
		assert_eq!(refused.unwrap_err().error, "server");
	}

	Rendering.cancel_passkey();
	assert_eq!(Rendering.provider_outcome(), None);

	// Outside a request there's no session to read two-step status from.
	assert_eq!(Rendering.mfa().await.unwrap_err().error, "server");
	for read in [
		Rendering.passkeys().await.map(|_| ()),
		Rendering.providers().await.map(|_| ()),
		Rendering.identities().await.map(|_| ()),
		Rendering.merge_offer().await.map(|_| ()),
		Rendering.wallet_settings().await.map(|_| ()),
		Rendering.wallets().await.map(|_| ()),
	] {
		assert_eq!(read.unwrap_err().error, "server");
	}
}

/// Wallets are asked to sign in the browser, not while the server renders.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn server_rendering_leaves_wallets_to_the_browser() {
	use super::server::Server as Rendering;

	for refused in [
		Rendering.wallet_apps().await.map(|_| ()),
		Rendering
			.link_wallet(LinkWalletInput::default())
			.await
			.map(|_| ()),
		Rendering
			.prove_wallet(ProveWalletInput {
				wallet: "Test Wallet".into(),
				address: leptoskit_db::Pubkey::from_bytes([1; 32]),
			})
			.await
			.map(|_| ()),
		Rendering
			.update_wallet(UpdateWallet {
				id: Id::new(),
				label: None,
				can_sign_in: None,
				is_public: None,
			})
			.await
			.map(|_| ()),
		Rendering.unlink_wallet(Id::new()).await,
		Rendering
			.wallet_sign_in("Test Wallet".into())
			.await
			.map(|_| ()),
	] {
		assert_eq!(refused.unwrap_err().error, "server");
	}
}

#[cfg(feature = "passkey")]
#[path = "../../tests/support/passkey.rs"]
mod soft_passkey;

/// What a browser does for sign-in, in memory: a software passkey shared by
/// every device that uses it, and a page that remembers where it was sent.
#[derive(Clone, Default)]
struct Page {
	#[cfg(feature = "passkey")]
	passkey: Arc<Mutex<Option<soft_passkey::SoftPasskey>>>,
	opened: Arc<Mutex<Option<String>>>,
	query: Arc<Mutex<Option<String>>>,
	cancelled: Arc<AtomicBool>,
	/// The wallet app "Test Wallet": a key made at test time, or none.
	wallet: Arc<Mutex<Option<ed25519_dalek::SigningKey>>>,
	/// Whether the wallet app declines to sign.
	declines: Arc<AtomicBool>,
}

impl Page {
	fn with_wallet() -> Self {
		let page = Self::default();
		let seed = crate::secrets::random::<32>().unwrap();
		*page.wallet.lock().unwrap() = Some(ed25519_dalek::SigningKey::from_bytes(&seed));
		page
	}

	fn address(&self) -> leptoskit_db::Pubkey {
		let key = self.wallet.lock().unwrap();
		leptoskit_db::Pubkey::from_bytes(key.as_ref().unwrap().verifying_key().to_bytes())
	}
}

impl protocol::Platform for Page {
	#[cfg(feature = "passkey")]
	fn create(
		&self,
		options: crate::CreationChallengeResponse,
	) -> BoxFuture<'static, Result<crate::RegisterPublicKeyCredential, ErrorResponse>> {
		let mut passkey = self.passkey.lock().unwrap();
		let device = passkey.get_or_insert_with(|| soft_passkey::SoftPasskey::new(ORIGIN));
		Box::pin(std::future::ready(Ok(device.register(&options))))
	}

	#[cfg(not(feature = "passkey"))]
	fn create(
		&self,
		_options: crate::CreationChallengeResponse,
	) -> BoxFuture<'static, Result<crate::RegisterPublicKeyCredential, ErrorResponse>> {
		Box::pin(std::future::ready(protocol::unsupported()))
	}

	#[cfg(feature = "passkey")]
	fn get(
		&self,
		options: crate::RequestChallengeResponse,
		autofill: bool,
	) -> BoxFuture<'static, Result<crate::PublicKeyCredential, ErrorResponse>> {
		let conditional = serde_json::to_value(&options).unwrap()["mediation"] == "conditional";
		assert_eq!(
			conditional, autofill,
			"autofill asks for conditional mediation"
		);
		let mut passkey = self.passkey.lock().unwrap();

		let answer = match passkey.as_mut() {
			Some(device) if device.pick(&options).is_some() => Ok(device.sign(&options)),
			_ => Err(error("cancelled", "no passkey")),
		};

		Box::pin(std::future::ready(answer))
	}

	#[cfg(not(feature = "passkey"))]
	fn get(
		&self,
		_options: crate::RequestChallengeResponse,
		_autofill: bool,
	) -> BoxFuture<'static, Result<crate::PublicKeyCredential, ErrorResponse>> {
		Box::pin(std::future::ready(protocol::unsupported()))
	}

	fn cancel(&self) {
		self.cancelled.store(true, Ordering::SeqCst);
	}

	fn open(&self, url: &str) -> Result<(), ErrorResponse> {
		*self.opened.lock().unwrap() = Some(url.to_owned());
		Ok(())
	}

	fn query(&self) -> Option<String> {
		self.query.lock().unwrap().clone()
	}

	fn wallet_apps(&self) -> BoxFuture<'static, Result<Vec<WalletApp>, ErrorResponse>> {
		let apps = self
			.wallet
			.lock()
			.unwrap()
			.iter()
			.map(|_| {
				WalletApp {
					name: "Test Wallet".into(),
					icon: "data:image/svg+xml,<svg/>".into(),
					sign_in: true,
				}
			})
			.collect();
		Box::pin(std::future::ready(Ok(apps)))
	}

	fn sign_with_wallet(
		&self,
		wallet: String,
		input: crate::SiwsInput,
	) -> BoxFuture<'static, Result<crate::WalletProof, ErrorResponse>> {
		use ed25519_dalek::Signer;

		let key = self.wallet.lock().unwrap();

		let answer = match key.as_ref() {
			_ if self.declines.load(Ordering::SeqCst) => Err(error("cancelled", "declined")),

			Some(key) if wallet == "Test Wallet" => {
				let address = leptoskit_db::Pubkey::from_bytes(key.verifying_key().to_bytes());

				match &input.address {
					Some(wanted) if *wanted != address.to_string() => {
						Err(error("wallet_failed", "another account"))
					}

					_ => {
						let message = input.message(&address);
						Ok(crate::WalletProof {
							address,
							signature: leptoskit_db::Signature::from_bytes(
								key.sign(message.as_bytes()).to_bytes(),
							),
							message,
						})
					}
				}
			}

			_ => Err(error("no_wallet", "no wallet by that name")),
		};

		Box::pin(std::future::ready(answer))
	}
}

impl Server {
	/// A browser on `page`.
	fn browser_on(&self, page: &Page) -> Device {
		let http = InMemory::new(self, true);
		let jar = Arc::clone(&http.jar);
		Device {
			api: Arc::new(
				Remote::new(ClientConfig::new(), http.clone(), Cookie).platform(page.clone()),
			),
			http,
			held: Box::new(move || jar.lock().unwrap().contains_key("__Host-session")),
		}
	}
}

#[cfg(feature = "passkey")]
#[tokio::test]
async fn a_browser_adds_passkeys_and_signs_in_with_them() {
	let server = Server::new().await;
	let page = Page::default();
	let laptop = server.browser_on(&page);
	sign_in(&server, &laptop).await;

	let added = laptop.api.add_passkey(Some("Laptop".into())).await.unwrap();
	assert_eq!(added.passkey.name, "Laptop");
	assert_eq!(added.session.aal, crate::Aal::Two);
	assert_eq!(added.recovery_codes.unwrap().codes.len(), 10);
	assert_eq!(session_id(&laptop).await, Some(added.session.id));
	let listed = laptop.api.passkeys().await.unwrap();
	assert_eq!(listed.len(), 1);
	laptop
		.api
		.rename_passkey(RenamePasskey {
			id: listed[0].id,
			name: "Work laptop".into(),
		})
		.await
		.unwrap();

	assert_eq!(laptop.api.passkeys().await.unwrap()[0].name, "Work laptop");

	// Another browser on the same device signs in with it, from autofill or
	// the dialog, without an email.
	let phone = server.browser_on(&page);
	let session = phone
		.api
		.passkey_sign_in(PasskeyMode::Autofill)
		.await
		.unwrap();
	assert_eq!(session.aal, crate::Aal::Two);
	assert!(
		!phone
			.http
			.jar
			.lock()
			.unwrap()
			.contains_key("__Host-passkey")
	);
	let other = server.browser_on(&page);
	other
		.api
		.passkey_sign_in(PasskeyMode::Prompt)
		.await
		.unwrap();
	other.api.cancel_passkey();
	assert!(page.cancelled.load(Ordering::SeqCst));

	// An emailed code now asks for the passkey second.
	let tablet = server.browser_on(&page);
	tablet.api.send_code(ADA.to_owned()).await.unwrap();
	let step = tablet
		.api
		.verify_code(EmailCode {
			email: ADA.to_owned(),
			code: server.code(),
		})
		.await
		.unwrap();
	let SignInStep::SecondFactor(prompt) = step else {
		panic!("a second factor");
	};
	assert_eq!(prompt.factors[0], Factor::Passkey);
	let session = tablet.api.complete_sign_in_with_passkey().await.unwrap();
	assert_eq!(session.aal, crate::Aal::Two);

	// Step-up with the passkey, then removing it.
	tablet.api.step_up_with_passkey().await.unwrap();
	tablet.api.remove_passkey(listed[0].id).await.unwrap();
	assert_eq!(tablet.api.passkeys().await.unwrap(), [] as [PasskeyInfo; 0]);
	assert_eq!(session_id(&laptop).await, None, "other devices signed out");

	// With no passkey left, the device has nothing to offer.
	let stranger = server.browser_on(&Page::default());
	assert_eq!(
		stranger
			.api
			.passkey_sign_in(PasskeyMode::Prompt)
			.await
			.unwrap_err()
			.error,
		"cancelled"
	);
}

#[tokio::test]
async fn a_native_app_without_its_plugins_says_unsupported() {
	let server = Server::new().await;
	let phone = server.native().device();
	sign_in(&server, &phone).await;
	phone.http.count("");

	for refused in [
		phone
			.api
			.passkey_sign_in(PasskeyMode::Autofill)
			.await
			.map(|_| ()),
		phone
			.api
			.passkey_sign_in(PasskeyMode::Prompt)
			.await
			.map(|_| ()),
		phone.api.add_passkey(None).await.map(|_| ()),
		phone.api.complete_sign_in_with_passkey().await.map(|_| ()),
		phone.api.step_up_with_passkey().await.map(|_| ()),
		phone
			.api
			.start_provider(
				StartProvider {
					provider: "google".into(),
					redirect_to: None,
				},
				false,
			)
			.await,
		phone
			.api
			.merge_accounts(MergeInput {
				passkey: true,
				confirm: true,
				..MergeInput::default()
			})
			.await,
	] {
		assert_eq!(refused.unwrap_err().error, "unsupported");
	}

	// With no provider sign-in waiting, there's nothing to continue.
	assert_eq!(
		phone
			.api
			.continue_provider_sign_up()
			.await
			.unwrap_err()
			.error,
		"sign_in_expired"
	);
	assert_eq!(phone.http.count(""), 0, "nothing was asked of the server");
	phone.api.cancel_passkey();
	assert_eq!(phone.api.provider_outcome(), None);

	// What only reads the server works the same everywhere.
	assert_eq!(phone.api.passkeys().await.unwrap(), [] as [PasskeyInfo; 0]);
	assert_eq!(
		phone.api.identities().await.unwrap(),
		[] as [IdentityInfo; 0]
	);
	assert_eq!(phone.api.merge_offer().await.unwrap(), None);
	phone.api.cancel_merge().await.unwrap();
	assert_eq!(
		phone
			.api
			.rename_passkey(RenamePasskey {
				id: Id::new(),
				name: "x".into()
			})
			.await
			.unwrap_err()
			.error,
		"not_found"
	);
	assert_eq!(
		phone.api.remove_passkey(Id::new()).await.unwrap_err().error,
		"step_up_required"
	);
	assert_eq!(
		phone
			.api
			.unlink_identity(Id::new())
			.await
			.unwrap_err()
			.error,
		"not_found"
	);
	assert_eq!(
		phone.api.providers().await.unwrap(),
		[] as [ProviderInfo; 0]
	);
}

#[tokio::test]
async fn a_browser_goes_to_providers_and_reads_where_it_came_back() {
	let server = Server::new().await;
	let page = Page::default();
	let browser = server.browser_on(&page);

	// This server has no providers; the request still goes out, and says so.
	let unknown = browser
		.api
		.start_provider(
			StartProvider {
				provider: "google".into(),
				redirect_to: None,
			},
			false,
		)
		.await
		.unwrap_err();
	assert_eq!(unknown.error, "unknown_provider");
	assert_eq!(page.opened.lock().unwrap().clone(), None);
	assert_eq!(
		browser
			.api
			.continue_provider_sign_up()
			.await
			.unwrap_err()
			.error,
		"sign_in_expired"
	);

	assert_eq!(browser.api.provider_outcome(), None);
	*page.query.lock().unwrap() = Some("?tab=1&auth=error&auth_error=cancelled".into());
	assert_eq!(
		browser.api.provider_outcome(),
		Some(ProviderOutcome::Failed("cancelled".into()))
	);
	*page.query.lock().unwrap() = Some("?auth=merge".into());
	assert_eq!(browser.api.provider_outcome(), Some(ProviderOutcome::Merge));
}

#[cfg(feature = "oauth")]
#[tokio::test]
async fn a_browser_is_sent_to_the_provider_with_a_binding_cookie() {
	let server = Server::new().await;
	let auth = Auth::builder(&server.db, Keys::single("test", [9; 32]))
		.mailer(server.mailbox.clone())
		.origin(ORIGIN)
		.provider(crate::Provider::google("google-client", "secret"))
		.build()
		.unwrap();
	let router = Router::new().merge(auth.router()).layer(auth.layer());
	let page = Page::default();
	let http = InMemory {
		router,
		..InMemory::new(&server, true)
	};
	let api = Remote::new(ClientConfig::new(), http.clone(), Cookie).platform(page.clone());

	assert_eq!(
		Api::providers(&api).await.unwrap(),
		[ProviderInfo {
			id: "google".into(),
			name: "Google".into()
		}]
	);
	Api::start_provider(
		&api,
		StartProvider {
			provider: "google".into(),
			redirect_to: Some("/settings".into()),
		},
		false,
	)
	.await
	.unwrap();
	let opened = page.opened.lock().unwrap().clone().unwrap();
	assert!(opened.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
	assert!(http.jar.lock().unwrap().contains_key("__Host-provider"));

	// Connecting needs someone signed in.
	let refused = Api::start_provider(
		&api,
		StartProvider {
			provider: "google".into(),
			redirect_to: None,
		},
		true,
	)
	.await
	.unwrap_err();
	assert_eq!(refused.error, "signed_out");
}

#[tokio::test]
async fn the_passkey_and_provider_scopes_call_the_configured_client() {
	let _turn = INSTALLED.lock().await;
	use leptos::prelude::Owner;
	use leptoskit_query::NetworkMode;
	use leptoskit_query::QueryClient;
	use leptoskit_query::QueryDefaults;
	use leptoskit_query::RefetchOn;
	use leptoskit_query::use_mutation_in;

	let _ = any_spawner::Executor::init_tokio();
	let owner = Owner::new();
	owner.set();
	let server = Server::new().await;
	let page = Page::default();
	let browser = server.browser_on(&page);
	install(Arc::clone(&browser.api));
	let client = QueryClient::new().with_defaults(
		QueryDefaults::new()
			.with_refetch_on_window_focus(RefetchOn::Never)
			.with_refetch_on_reconnect(RefetchOn::Never)
			.with_network_mode(NetworkMode::Always),
	);

	client.provide();
	sign_in(&server, &browser).await;

	assert_eq!(use_providers().await, Ok(Vec::new()));
	assert_eq!(use_identities().await, Ok(Vec::new()));
	assert_eq!(use_passkeys().await, Ok(Vec::new()));
	assert_eq!(use_merge_offer().await, Ok(None));
	assert_eq!(provider_outcome(), None);
	cancel_passkey_sign_in();
	assert!(page.cancelled.load(Ordering::SeqCst));
	use_mutation_in(client.clone(), cancel_merge())
		.mutate_async(())
		.await
		.unwrap();

	for refused in [
		use_mutation_in(client.clone(), sign_in_with_provider())
			.mutate_async(StartProvider {
				provider: "google".into(),
				redirect_to: None,
			})
			.await,
		use_mutation_in(client.clone(), link_provider())
			.mutate_async(StartProvider {
				provider: "google".into(),
				redirect_to: None,
			})
			.await,
	] {
		assert_eq!(refused.unwrap_err().error, "unknown_provider");
	}

	assert_eq!(
		use_mutation_in(client.clone(), continue_provider_sign_up())
			.mutate_async(())
			.await
			.unwrap_err()
			.error,
		"sign_in_expired"
	);
	assert_eq!(
		use_mutation_in(client.clone(), unlink_identity())
			.mutate_async(Id::new())
			.await
			.unwrap_err()
			.error,
		"not_found"
	);
	assert_eq!(
		use_mutation_in(client.clone(), merge_accounts())
			.mutate_async(MergeInput::default())
			.await
			.unwrap_err()
			.error,
		"unsupported"
	);

	#[cfg(feature = "passkey")]
	{
		let added = use_mutation_in(client.clone(), add_passkey())
			.mutate_async(None)
			.await
			.unwrap();
		assert_eq!(
			client.get_query_data(&session(), &()),
			Some(Some(added.session.clone()))
		);
		assert_eq!(use_passkeys().await.unwrap().len(), 1);
		use_mutation_in(client.clone(), rename_passkey())
			.mutate_async(RenamePasskey {
				id: added.passkey.id,
				name: "Mine".into(),
			})
			.await
			.unwrap();
		let stepped = use_mutation_in(client.clone(), step_up_with_passkey())
			.mutate_async(())
			.await
			.unwrap();
		assert_eq!(client.get_query_data(&session(), &()), Some(Some(stepped)));

		use_mutation_in(client.clone(), remove_passkey())
			.mutate_async(added.passkey.id)
			.await
			.unwrap();
		let signed_in = use_mutation_in(client.clone(), passkey_sign_in())
			.mutate_async(PasskeyMode::Prompt)
			.await;
		assert_eq!(signed_in.unwrap_err().error, "passkey_failed");
		assert_eq!(
			use_mutation_in(client.clone(), complete_sign_in_with_passkey())
				.mutate_async(())
				.await
				.unwrap_err()
				.error,
			"sign_in_expired"
		);
	}
	// The next test may expect no client installed yet.
	*CLIENT.write().unwrap() = None;
}

#[tokio::test]
async fn a_device_without_passkeys_or_pages_says_so() {
	use protocol::Platform as _;

	let none = protocol::NoPlatform;
	let options: crate::RequestChallengeResponse = serde_json::from_value(serde_json::json!({
		"publicKey": { "challenge": "AAAA", "rpId": "notes.example", "allowCredentials": [], "userVerification": "required" }
	}))
	.unwrap();
	assert_eq!(
		none.get(options, false).await.unwrap_err().error,
		"unsupported"
	);
	let created: crate::CreationChallengeResponse = serde_json::from_value(serde_json::json!({
		"publicKey": {
			"rp": { "id": "notes.example", "name": "Notes" },
			"user": { "id": "AAAA", "name": "ada", "displayName": "Ada" },
			"challenge": "AAAA",
			"pubKeyCredParams": [],
		}
	}))
	.unwrap();
	assert_eq!(none.create(created).await.unwrap_err().error, "unsupported");
	assert_eq!(
		none.open("https://example.com").unwrap_err().error,
		"unsupported"
	);
	none.cancel();
	assert_eq!(none.query(), None);

	assert_eq!(none.wallet_apps().await.unwrap_err().error, "unsupported");
	let signed = none
		.sign_with_wallet("Phantom".into(), crate::SiwsInput::default())
		.await;
	assert_eq!(signed.unwrap_err().error, "unsupported");
	assert!(!none.passkeys() && !none.autofill() && !none.has_apple_id());
	assert_eq!(none.begin_browser().await.unwrap_err().error, "unsupported");
	let session = protocol::BrowserSession {
		id: 1,
		return_to: "com.notes.app:/auth/native/return".into(),
	};
	assert_eq!(
		none.browse(session, "https://sso.example/".into())
			.await
			.unwrap_err()
			.error,
		"unsupported"
	);
	assert_eq!(
		none.apple_id("nonce".into()).await.unwrap_err().error,
		"unsupported"
	);
	let input = MergeInput {
		code: Some("123456".into()),
		..MergeInput::default()
	};
	assert!(!format!("{input:?}").contains("123456"));
}

#[cfg(feature = "solana")]
#[tokio::test]
async fn a_browser_links_a_wallet_and_signs_in_with_it() {
	let server = Server::new().await;
	let page = Page::with_wallet();
	let laptop = server.browser_on(&page);
	let settings = laptop.api.wallet_settings().await.unwrap();
	assert!(settings.enabled && settings.sign_in);
	assert_eq!(settings.chain_id, "mainnet");
	assert_eq!(
		laptop.api.wallet_apps().await.unwrap()[0].name,
		"Test Wallet"
	);
	sign_in(&server, &laptop).await;

	let linked = laptop
		.api
		.link_wallet(LinkWalletInput {
			wallet: "Test Wallet".into(),
			label: Some("Treasury".into()),
		})
		.await
		.unwrap();
	let WalletLinked::Linked(wallet) = linked else {
		panic!("linked: {linked:?}");
	};
	assert_eq!(wallet.address, page.address());
	assert_eq!(wallet.label.as_deref(), Some("Treasury"));
	assert_eq!(laptop.api.wallets().await.unwrap().len(), 1);

	// A wallet app that isn't there, or a person who declines.
	let missing = laptop
		.api
		.link_wallet(LinkWalletInput {
			wallet: "Elsewhere".into(),
			label: None,
		})
		.await
		.unwrap_err();
	assert_eq!(missing.error, "no_wallet");
	page.declines.store(true, Ordering::SeqCst);
	let declined = laptop
		.api
		.prove_wallet(ProveWalletInput {
			wallet: "Test Wallet".into(),
			address: page.address(),
		})
		.await
		.unwrap_err();
	assert_eq!(declined.error, "cancelled");
	page.declines.store(false, Ordering::SeqCst);
	let proved = laptop
		.api
		.prove_wallet(ProveWalletInput {
			wallet: "Test Wallet".into(),
			address: page.address(),
		})
		.await
		.unwrap();
	assert!(proved.last_proved_at >= wallet.last_proved_at);

	let updated = laptop
		.api
		.update_wallet(UpdateWallet {
			id: wallet.id,
			label: None,
			can_sign_in: Some(true),
			is_public: None,
		})
		.await
		.unwrap();
	assert!(updated.can_sign_in);

	// Another browser signs in with it, as one factor.
	let phone = server.browser_on(&page);
	let step = phone
		.api
		.wallet_sign_in("Test Wallet".into())
		.await
		.unwrap();
	let SignInStep::SignedIn(session) = step else {
		panic!("signed in");
	};
	assert_eq!(session.aal, crate::Aal::One);
	assert!(!phone.http.jar.lock().unwrap().contains_key("__Host-wallet"));
	assert!((phone.held)());

	laptop.api.unlink_wallet(wallet.id).await.unwrap();
	assert_eq!(laptop.api.wallets().await.unwrap(), [] as [WalletInfo; 0]);
	let refused = server
		.browser_on(&page)
		.api
		.wallet_sign_in("Test Wallet".into())
		.await
		.unwrap_err();
	assert_eq!(refused.error, "unknown_wallet");
}

#[tokio::test]
async fn a_native_app_manages_wallets_but_cannot_ask_one_to_sign_yet() {
	let server = Server::new().await;
	let page = Page::with_wallet();
	let browser = server.browser_on(&page);
	sign_in(&server, &browser).await;
	browser
		.api
		.link_wallet(LinkWalletInput {
			wallet: "Test Wallet".into(),
			label: None,
		})
		.await
		.unwrap();

	let phone = server.native().device();
	sign_in(&server, &phone).await;
	phone.http.count("");

	for refused in [
		phone.api.wallet_apps().await.map(|_| ()),
		phone
			.api
			.link_wallet(LinkWalletInput::default())
			.await
			.map(|_| ()),
		phone
			.api
			.prove_wallet(ProveWalletInput {
				wallet: "Test Wallet".into(),
				address: page.address(),
			})
			.await
			.map(|_| ()),
		phone
			.api
			.wallet_sign_in("Test Wallet".into())
			.await
			.map(|_| ()),
	] {
		assert_eq!(refused.unwrap_err().error, "unsupported");
	}

	assert_eq!(phone.http.count(""), 0, "nothing was asked of the server");
	// The same functions that only talk to the server work.
	assert!(phone.api.wallet_settings().await.unwrap().enabled);
	let wallets = phone.api.wallets().await.unwrap();
	assert_eq!(wallets.len(), 1);
	phone
		.api
		.update_wallet(UpdateWallet {
			id: wallets[0].id,
			label: Some("Phone label".into()),
			can_sign_in: None,
			is_public: None,
		})
		.await
		.unwrap();
	phone.api.unlink_wallet(wallets[0].id).await.unwrap();
	assert_eq!(phone.api.wallets().await.unwrap(), [] as [WalletInfo; 0]);
}

#[cfg(feature = "solana")]
#[tokio::test]
async fn the_wallet_scopes_call_the_configured_client() {
	let _turn = INSTALLED.lock().await;
	use leptos::prelude::Owner;
	use leptoskit_query::NetworkMode;
	use leptoskit_query::QueryClient;
	use leptoskit_query::QueryDefaults;
	use leptoskit_query::RefetchOn;
	use leptoskit_query::use_mutation_in;

	let _ = any_spawner::Executor::init_tokio();
	let owner = Owner::new();
	owner.set();
	let server = Server::new().await;
	let page = Page::with_wallet();
	let browser = server.browser_on(&page);
	install(Arc::clone(&browser.api));
	let client = QueryClient::new().with_defaults(
		QueryDefaults::new()
			.with_refetch_on_window_focus(RefetchOn::Never)
			.with_refetch_on_reconnect(RefetchOn::Never)
			.with_network_mode(NetworkMode::Always),
	);

	client.provide();
	sign_in(&server, &browser).await;

	assert!(use_wallet_settings().await.unwrap().sign_in);
	assert_eq!(use_wallet_apps().await.unwrap().len(), 1);
	assert_eq!(use_wallets().await, Ok(Vec::new()));

	let linked = use_mutation_in(client.clone(), link_wallet())
		.mutate_async(LinkWalletInput {
			wallet: "Test Wallet".into(),
			label: None,
		})
		.await
		.unwrap();
	let WalletLinked::Linked(wallet) = linked else {
		panic!("linked: {linked:?}");
	};
	use_mutation_in(client.clone(), prove_wallet())
		.mutate_async(ProveWalletInput {
			wallet: "Test Wallet".into(),
			address: page.address(),
		})
		.await
		.unwrap();
	let updated = use_mutation_in(client.clone(), update_wallet())
		.mutate_async(UpdateWallet {
			id: wallet.id,
			label: None,
			can_sign_in: Some(true),
			is_public: None,
		})
		.await
		.unwrap();
	assert!(updated.can_sign_in);

	let step = use_mutation_in(client.clone(), wallet_sign_in())
		.mutate_async("Test Wallet".into())
		.await
		.unwrap();
	let SignInStep::SignedIn(signed_in) = step else {
		panic!("signed in");
	};
	assert_eq!(
		client.get_query_data(&session(), &()),
		Some(Some(signed_in))
	);

	use_mutation_in(client.clone(), unlink_wallet())
		.mutate_async(wallet.id)
		.await
		.unwrap();
	assert_eq!(use_wallets().await, Ok(Vec::new()));
	// The next test may expect no client installed yet.
	*CLIENT.write().unwrap() = None;
}

#[cfg(feature = "oauth")]
#[path = "../../tests/support/oidc.rs"]
mod fake_idp;

/// The native app's bundle ID, which Sign in with Apple on the device
/// issues ID tokens to.
#[cfg(feature = "oauth")]
const BUNDLE: &str = "com.notes.App";

/// What a native app's device does for sign-in, in memory: the software
/// passkey of a [`Page`], a system browser that signs in at the fake
/// provider as `claims` and follows this site's redirect back to the app,
/// and Sign in with Apple that mints an ID token.
#[cfg(feature = "oauth")]
#[derive(Clone)]
struct Phone {
	page: Page,
	router: Router,
	idp: fake_idp::FakeIdp,
	claims: Arc<Mutex<serde_json::Value>>,
	/// Whether it has Sign in with Apple of its own.
	apple: bool,
	/// The person closes the browser instead of signing in.
	closes: Arc<AtomicBool>,
	/// Something other than the provider comes back to the app.
	forges: Arc<AtomicBool>,
	returns: Arc<Mutex<Vec<String>>>,
}

#[cfg(feature = "oauth")]
impl protocol::Platform for Phone {
	fn create(
		&self,
		options: crate::CreationChallengeResponse,
	) -> BoxFuture<'static, Result<crate::RegisterPublicKeyCredential, ErrorResponse>> {
		self.page.create(options)
	}

	fn get(
		&self,
		options: crate::RequestChallengeResponse,
		autofill: bool,
	) -> BoxFuture<'static, Result<crate::PublicKeyCredential, ErrorResponse>> {
		assert!(!autofill, "a native app never asks for autofill");
		self.page.get(options, false)
	}

	fn cancel(&self) {
		self.page.cancel();
	}

	fn open(&self, _url: &str) -> Result<(), ErrorResponse> {
		panic!("a native app never navigates a page");
	}

	fn query(&self) -> Option<String> {
		None
	}

	fn autofill(&self) -> bool {
		false
	}

	fn begin_browser(&self) -> BoxFuture<'static, Result<protocol::BrowserSession, ErrorResponse>> {
		self.returns
			.lock()
			.unwrap()
			.push("http://127.0.0.1:49152/auth/native/return".into());
		Box::pin(std::future::ready(Ok(protocol::BrowserSession {
			id: 1,
			return_to: "http://127.0.0.1:49152/auth/native/return".into(),
		})))
	}

	fn browse(
		&self,
		session: protocol::BrowserSession,
		url: String,
	) -> BoxFuture<'static, Result<String, ErrorResponse>> {
		let this = self.clone();
		Box::pin(async move {
			if this.closes.load(Ordering::SeqCst) {
				return Err(error("cancelled", "closed"));
			}

			let claims = this.claims.lock().unwrap().clone();
			let back = this.idp.authorize(&url, claims);
			let redirect_uri = url::Url::parse(&url)
				.unwrap()
				.query_pairs()
				.find(|(name, _)| name == "redirect_uri")
				.unwrap()
				.1
				.into_owned();
			let path = redirect_uri.trim_start_matches(ORIGIN).to_owned();
			let query = url::form_urlencoded::Serializer::new(String::new())
				.extend_pairs(&back)
				.finish();
			// The system browser has no cookies of this site's.
			let request = if path.ends_with("/apple") {
				Request::post(path)
					.header(header::ORIGIN, "https://appleid.apple.com")
					.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
					.body(Body::from(query))
			} else {
				Request::get(format!("{path}?{query}")).body(Body::empty())
			};

			let response = this.router.oneshot(request.unwrap()).await.unwrap();
			assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
			let location = response.headers()[header::LOCATION]
				.to_str()
				.unwrap()
				.to_owned();
			assert!(location.starts_with(&session.return_to), "{location}");

			if this.forges.load(Ordering::SeqCst) {
				return Ok(format!("{}?state=forged&code=stolen", session.return_to));
			}

			Ok(location)
		})
	}

	fn has_apple_id(&self) -> bool {
		self.apple
	}

	fn apple_id(
		&self,
		nonce: String,
	) -> BoxFuture<'static, Result<protocol::AppleId, ErrorResponse>> {
		if !self.apple {
			return Box::pin(std::future::ready(protocol::unsupported()));
		}

		let claims = self.claims.lock().unwrap().clone();
		let id_token = self.idp.apple_id_token(BUNDLE, &nonce, claims);
		Box::pin(std::future::ready(Ok(protocol::AppleId {
			id_token,
			user: Some(r#"{"name":{"firstName":"Ada","lastName":"Lovelace"}}"#.into()),
		})))
	}
}

/// A server with providers and the native app's bundle ID, the fake
/// provider it reaches, and a native app on a [`Phone`].
#[cfg(feature = "oauth")]
async fn provider_server() -> (Server, fake_idp::FakeIdp) {
	let db = leptoskit_db::testing::sqlite().await;
	let mailbox = Mailbox::new();
	let idp = fake_idp::FakeIdp::new();
	let (pem, public) = fake_idp::apple_key();
	idp.trust_apple_key(public);
	let auth = Auth::builder(&db, Keys::single("test", [9; 32]))
		.mailer(mailbox.clone())
		.origin(ORIGIN)
		.native_apps(crate::NativeApps {
			package_id: BUNDLE.into(),
			apple_team_id: Some("ABCDE12345".into()),
			android_certificates: Vec::new(),
		})
		.provider(
			crate::Provider::oidc(
				"company",
				"Company SSO",
				fake_idp::ISSUER,
				"notes-client",
				Some(fake_idp::SECRET.into()),
			)
			.unwrap(),
		)
		.provider(crate::Provider::google("google-client", fake_idp::SECRET))
		.provider(crate::Provider::apple("com.notes.web", "TEAM1", "KEY1", &pem).unwrap())
		.provider_http(idp.clone())
		.build()
		.unwrap();
	let router = Router::new().merge(auth.router()).layer(auth.layer());
	(
		Server {
			auth,
			router,
			mailbox,
			db,
		},
		idp,
	)
}

#[cfg(feature = "oauth")]
impl Server {
	fn phone(&self, idp: &fake_idp::FakeIdp, apple: bool) -> (Native, Phone) {
		self.phone_on(idp, apple, Page::default())
	}

	/// A phone whose passkeys are `page`'s.
	fn phone_on(&self, idp: &fake_idp::FakeIdp, apple: bool, page: Page) -> (Native, Phone) {
		let phone = Phone {
			page,
			router: self.router.clone(),
			idp: idp.clone(),
			claims: Arc::new(Mutex::new(serde_json::json!({
				"sub": "u-1", "email": ADA, "email_verified": true, "name": "Ada Lovelace",
			}))),
			apple,
			closes: Arc::default(),
			forges: Arc::default(),
			returns: Arc::default(),
		};
		let http = InMemory::new(self, false);
		let clock = Clock::default();
		let now = clock.clone();
		let config = ClientConfig::new()
			.origin(ORIGIN)
			.device_name("Ada's phone");
		let native = Native {
			api: Arc::new(
				Remote::new(
					config,
					http.clone(),
					TokenKeeper::new(Memory::default(), move || now.0.load(Ordering::SeqCst)),
				)
				.platform(phone.clone()),
			),
			http,
			clock,
		};
		(native, phone)
	}
}

#[cfg(feature = "oauth")]
fn provider(id: &str) -> StartProvider {
	StartProvider {
		provider: id.into(),
		redirect_to: None,
	}
}

#[cfg(feature = "oauth")]
#[tokio::test]
async fn a_native_app_signs_in_with_providers_in_the_system_browser() {
	let (server, idp) = provider_server().await;
	let (native, phone) = server.phone(&idp, false);
	let api = &native.api;

	Api::start_provider(&**api, provider("company"), false)
		.await
		.unwrap();
	assert_eq!(
		Api::provider_outcome(&**api),
		Some(ProviderOutcome::SignedIn)
	);
	assert!(native.store().held().is_some(), "the refresh token is kept");
	let session = Api::session(&**api).await.unwrap().unwrap();
	let devices = Api::devices(&**api).await.unwrap();
	assert_eq!(devices[0].session.id, session.id);
	assert_eq!(devices[0].session.device, "Ada's phone");
	assert_eq!(phone.returns.lock().unwrap().len(), 1);

	// Connecting another provider, from the signed-in app.
	*phone.claims.lock().unwrap() = serde_json::json!({
		"sub": "g-1", "email": "ada@gmail.example", "email_verified": true,
	});
	Api::start_provider(&**api, provider("google"), true)
		.await
		.unwrap();
	assert_eq!(Api::provider_outcome(&**api), Some(ProviderOutcome::Linked));
	assert_eq!(Api::identities(&**api).await.unwrap().len(), 2);

	// Closing the browser, and an answer that isn't the provider's.
	phone.closes.store(true, Ordering::SeqCst);
	Api::start_provider(&**api, provider("company"), false)
		.await
		.unwrap();
	assert_eq!(
		Api::provider_outcome(&**api),
		Some(ProviderOutcome::Failed("cancelled".into()))
	);
	phone.closes.store(false, Ordering::SeqCst);
	phone.forges.store(true, Ordering::SeqCst);
	Api::start_provider(&**api, provider("company"), false)
		.await
		.unwrap();
	assert_eq!(
		Api::provider_outcome(&**api),
		Some(ProviderOutcome::Failed("sign_in_expired".into()))
	);
	phone.forges.store(false, Ordering::SeqCst);

	// A new person whose address is Ada's chooses a separate account.
	let (other, other_phone) = server.phone(&idp, false);
	*other_phone.claims.lock().unwrap() = serde_json::json!({
		"sub": "g-2", "email": ADA, "email_verified": true,
	});
	Api::start_provider(&*other.api, provider("google"), false)
		.await
		.unwrap();
	assert_eq!(
		Api::provider_outcome(&*other.api),
		Some(ProviderOutcome::EmailInUse)
	);
	assert!(other.store().held().is_none());
	let separate = Api::continue_provider_sign_up(&*other.api).await.unwrap();
	assert_ne!(separate.account_id, session.account_id);
	assert!(other.store().held().is_some());
	assert_eq!(
		Api::continue_provider_sign_up(&*other.api)
			.await
			.unwrap_err()
			.error,
		"sign_in_expired",
		"the choice is made once"
	);

	// An unknown provider is refused before anything opens.
	let returns = phone.returns.lock().unwrap().len();
	let unknown = Api::start_provider(&**api, provider("nope"), false)
		.await
		.unwrap_err();
	assert_eq!(unknown.error, "unknown_provider");
	assert_eq!(phone.returns.lock().unwrap().len(), returns + 1);
}

#[cfg(feature = "oauth")]
#[tokio::test]
async fn a_native_app_signs_in_with_apple_on_the_device() {
	let (server, idp) = provider_server().await;
	let (native, phone) = server.phone(&idp, true);

	Api::start_provider(&*native.api, provider("apple"), false)
		.await
		.unwrap();
	assert_eq!(
		Api::provider_outcome(&*native.api),
		Some(ProviderOutcome::SignedIn)
	);
	assert!(
		phone.returns.lock().unwrap().is_empty(),
		"no browser was needed"
	);
	let identities = Api::identities(&*native.api).await.unwrap();
	assert_eq!(identities[0].provider, "apple");
	assert_eq!(identities[0].name.as_deref(), Some("Ada Lovelace"));

	// Other providers still go through the browser on the same device.
	*phone.claims.lock().unwrap() = serde_json::json!({ "sub": "c-1" });
	Api::start_provider(&*native.api, provider("company"), false)
		.await
		.unwrap();
	assert_eq!(
		Api::provider_outcome(&*native.api),
		Some(ProviderOutcome::SignedIn)
	);
	assert_eq!(phone.returns.lock().unwrap().len(), 1);

	// A device without Sign in with Apple of its own uses Apple's web
	// sign-in in the system browser.
	let (android, android_phone) = server.phone(&idp, false);
	*android_phone.claims.lock().unwrap() = serde_json::json!({ "sub": "a-9" });
	Api::start_provider(&*android.api, provider("apple"), false)
		.await
		.unwrap();
	assert_eq!(
		Api::provider_outcome(&*android.api),
		Some(ProviderOutcome::SignedIn)
	);
	assert_eq!(android_phone.returns.lock().unwrap().len(), 1);
}

#[cfg(all(feature = "oauth", feature = "passkey"))]
#[tokio::test]
async fn a_native_app_makes_and_uses_passkeys_with_the_device() {
	let (server, idp) = provider_server().await;
	let (native, phone) = server.phone(&idp, true);
	let device = native.device();
	sign_in(&server, &device).await;

	let added = device.api.add_passkey(Some("iPhone".into())).await.unwrap();
	assert_eq!(added.session.aal, crate::Aal::Two);
	assert!(native.store().held().is_some(), "rotated tokens are kept");

	// Autofill isn't offered, and asks the server nothing.
	native.http.count("");
	assert_eq!(
		device
			.api
			.passkey_sign_in(PasskeyMode::Autofill)
			.await
			.unwrap_err()
			.error,
		"unsupported"
	);
	assert_eq!(native.http.count(""), 0);

	// Another app on a device with the same passkey signs in with it.
	let (other, _) = server.phone_on(&idp, true, phone.page.clone());
	let session = other
		.device()
		.api
		.passkey_sign_in(PasskeyMode::Prompt)
		.await
		.unwrap();
	assert_eq!(session.aal, crate::Aal::Two);
	assert!(other.store().held().is_some());

	let stepped = device.api.step_up_with_passkey().await.unwrap();
	assert!(stepped.mfa_at.is_some());
	device.api.cancel_passkey();
	assert!(phone.page.cancelled.load(Ordering::SeqCst));
}
