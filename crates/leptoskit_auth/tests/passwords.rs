//! Passwords, against both databases: off unless turned on, the rules for
//! new ones, signing in, the pause after too many wrong ones, resets
//! through emailed codes that wait for a second factor, re-hashing, and
//! the breach check.

#![cfg(all(feature = "postgres", feature = "sqlite", feature = "password"))]

#[path = "../../leptoskit_db/tests/support/postgres.rs"]
mod postgres;
#[path = "support/totp.rs"]
mod totp;

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use leptoskit_auth::Account;
use leptoskit_auth::Auth;
use leptoskit_auth::AuthError;
use leptoskit_auth::BreachFuture;
use leptoskit_auth::BreachedPasswords;
use leptoskit_auth::Event;
use leptoskit_auth::EventKind;
use leptoskit_auth::Keys;
use leptoskit_auth::Limit;
use leptoskit_auth::Limits;
use leptoskit_auth::Mailbox;
use leptoskit_auth::Password;
use leptoskit_auth::PasswordOptions;
use leptoskit_auth::PasswordProblem;
use leptoskit_auth::RequestInfo;
use leptoskit_auth::SignIn;
use leptoskit_auth::SignInStep;
use leptoskit_db::Connection;
use leptoskit_db::Db;
use leptoskit_db::Dialect;
use leptoskit_db::Id;
use leptoskit_db::Statement;
use leptoskit_db::Value;
use postgres::PostgresCluster;
use totp::totp;

const GOOD: &str = "correct horse battery staple";
const BETTER: &str = "a much longer and newer phrase";

async fn databases(cluster: Option<&PostgresCluster>) -> Vec<Db> {
	let mut databases = vec![Db::connect("sqlite::memory:").await.unwrap()];

	if let Some(cluster) = cluster {
		databases.push(Db::connect(&cluster.database()).await.unwrap());
	}

	for db in &databases {
		leptoskit_db::testing::create_tables(db).await.unwrap();
	}

	databases
}

fn request() -> RequestInfo {
	RequestInfo {
		ip: Some("203.0.113.9".parse().unwrap()),
		user_agent: Some("Firefox/131.0".into()),
		..RequestInfo::default()
	}
}

fn auth(db: &Db, mailbox: &Mailbox, options: Option<PasswordOptions>) -> Auth {
	let mut builder = Auth::builder(db, Keys::single("test", [7; 32]))
		.app_name("Notes App")
		.mailer(mailbox.clone())
		.limits(Limits {
			failures_before_lock: Limit::new(3, Duration::from_hours(24)),
			lock_for: Duration::from_hours(1),
			..Limits::default()
		});

	if let Some(options) = options {
		builder = builder.passwords(options);
	}

	builder.build().unwrap()
}

fn code(mailbox: &Mailbox, address: &str) -> String {
	let mail = mailbox.latest_to(address).unwrap();
	mail.subject.rsplit(' ').next().unwrap().to_owned()
}

async fn sign_in(auth: &Auth, mailbox: &Mailbox, address: &str) -> SignIn {
	auth.send_email_code(address, &request()).await.unwrap();
	auth.verify_email_code(address, &code(mailbox, address), &request())
		.await
		.unwrap()
		.signed_in()
		.unwrap()
}

async fn events(db: &Db, account: Id<Account>) -> Vec<EventKind> {
	Event::query()
		.filter(|event| event.account_id.eq(account))
		.order_by(|event| event.created_at.asc())
		.all(&db.system("test"))
		.await
		.unwrap()
		.into_iter()
		.map(|event| event.kind)
		.collect()
}

async fn live_sessions(db: &Db, account: Id<Account>) -> u64 {
	leptoskit_auth::Session::query()
		.filter(|session| session.account_id.eq(account) & session.revoked_at.is_null())
		.count(&db.system("test"))
		.await
		.unwrap()
}

#[tokio::test]
async fn passwords_are_off_unless_turned_on() {
	let db = leptoskit_db::testing::sqlite().await;
	let mailbox = Mailbox::new();
	let auth = auth(&db, &mailbox, None);
	assert!(!auth.password_settings().enabled);
	let refused = auth
		.sign_in_with_password("ada@example.com", GOOD, &request())
		.await
		.unwrap_err();
	assert!(matches!(refused, AuthError::Unsupported(_)), "{refused}");
	let me = sign_in(&auth, &mailbox, "ada@example.com")
		.await
		.signed_in();
	assert!(matches!(
		auth.set_password(&me, GOOD, &request()).await,
		Err(AuthError::Unsupported(_))
	));

	let on = self::auth(&db, &mailbox, Some(PasswordOptions::default()));
	let settings = on.password_settings();
	assert!(settings.enabled);
	assert_eq!((settings.min_length, settings.max_length), (15, 256));
	assert!(
		Auth::builder(&db, Keys::single("test", [7; 32]))
			.mailer(Mailbox::new())
			.passwords(PasswordOptions {
				min_length: 7,
				..PasswordOptions::default()
			})
			.build()
			.is_err()
	);
}

#[tokio::test]
async fn people_set_a_password_and_sign_in_with_it() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox, Some(PasswordOptions::default()));
		let laptop = sign_in(&auth, &mailbox, "ada.lovelace@example.com").await;
		let me = laptop.signed_in();
		let account = me.account;

		for (weak, problem) in [
			("short", PasswordProblem::TooShort { min: 15 }),
			("ada.lovelace2026", PasswordProblem::Personal),
			("notes app 123456", PasswordProblem::Personal),
		] {
			let refused = auth.set_password(&me, weak, &request()).await.unwrap_err();
			assert!(
				matches!(refused, AuthError::WeakPassword(found) if found == problem),
				"{weak}: {refused}"
			);
		}

		auth.set_password(&me, GOOD, &request()).await.unwrap();
		assert!(auth.mfa_status(account).await.unwrap().password);
		let stored = Password::query()
			.filter(|password| password.account_id.eq(account))
			.first(&db.system("test"))
			.await
			.unwrap()
			.unwrap();
		assert!(stored.hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
		assert!(!stored.hash.contains(GOOD));

		// Signing in with it; the address is matched however it's cased.
		let phone = auth
			.sign_in_with_password("Ada.Lovelace@example.com", GOOD, &request())
			.await
			.unwrap()
			.signed_in()
			.unwrap();
		assert_eq!(phone.session.account_id, account);
		assert_eq!(phone.session.aal, leptoskit_auth::Aal::One);

		// A wrong password and an unknown address answer the same.
		for (address, password) in [
			("ada.lovelace@example.com", "not the right password"),
			("nobody@example.com", GOOD),
		] {
			assert!(matches!(
				auth.sign_in_with_password(address, password, &request())
					.await,
				Err(AuthError::InvalidPassword)
			));
		}

		// Changing it ends the other sessions; the old one stops working.
		assert_eq!(live_sessions(&db, account).await, 2);
		auth.set_password(&me, BETTER, &request()).await.unwrap();
		assert_eq!(live_sessions(&db, account).await, 1);
		assert!(matches!(
			auth.sign_in_with_password("ada.lovelace@example.com", GOOD, &request())
				.await,
			Err(AuthError::InvalidPassword)
		));
		assert!(
			mailbox
				.latest_to("ada.lovelace@example.com")
				.unwrap()
				.text
				.contains("every other device was signed out")
		);

		// Removing it leaves emailed codes working.
		auth.remove_password(&me, &request()).await.unwrap();
		assert!(!auth.mfa_status(account).await.unwrap().password);
		assert!(matches!(
			auth.remove_password(&me, &request()).await,
			Err(AuthError::NotFound)
		));
		let kinds = events(&db, account).await;

		for kind in [
			EventKind::PasswordSet,
			EventKind::PasswordChanged,
			EventKind::PasswordRemoved,
		] {
			assert!(kinds.contains(&kind), "{kind:?}");
		}
	}
}

#[tokio::test]
async fn too_many_wrong_passwords_pause_only_password_sign_in() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox, Some(PasswordOptions::default()));
		let me = sign_in(&auth, &mailbox, "grace@example.com")
			.await
			.signed_in();
		auth.set_password(&me, GOOD, &request()).await.unwrap();

		// Three wrong guesses are allowed; the fourth pauses, for an address
		// with an account and one without alike.
		for address in ["grace@example.com", "nobody@example.com"] {
			for _ in 0..3 {
				assert!(matches!(
					auth.sign_in_with_password(address, "a wrong guess, again", &request())
						.await,
					Err(AuthError::InvalidPassword)
				));
			}

			assert!(matches!(
				auth.sign_in_with_password(address, "a wrong guess, again", &request())
					.await,
				Err(AuthError::InvalidPassword)
			));
			let paused = auth
				.sign_in_with_password(address, GOOD, &request())
				.await
				.unwrap_err();
			assert!(
				matches!(paused, AuthError::RateLimited { retry_after } if retry_after > Duration::from_mins(50)),
				"{address}: {paused}"
			);
		}

		// Only the account was told, and it isn't locked: an emailed code
		// still signs in.
		assert!(
			mailbox
				.latest_to("grace@example.com")
				.unwrap()
				.subject
				.contains("paused")
		);
		assert!(mailbox.latest_to("nobody@example.com").is_none());
		assert!(
			events(&db, me.account)
				.await
				.contains(&EventKind::PasswordPaused)
		);
		sign_in(&auth, &mailbox, "grace@example.com").await;
	}
}

#[tokio::test]
async fn a_reset_ends_every_session_and_waits_for_a_second_factor() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox, Some(PasswordOptions::default()));
		let first = sign_in(&auth, &mailbox, "hedy@example.com").await;
		let account = first.session.account_id;
		auth.set_password(&first.signed_in(), GOOD, &request())
			.await
			.unwrap();

		// A refused password leaves the code working.
		auth.send_email_code("hedy@example.com", &request())
			.await
			.unwrap();
		let emailed = code(&mailbox, "hedy@example.com");
		assert!(matches!(
			auth.reset_password("hedy@example.com", &emailed, "password1234", &request())
				.await,
			Err(AuthError::WeakPassword(PasswordProblem::TooShort { .. }))
		));
		let step = auth
			.reset_password("hedy@example.com", &emailed, BETTER, &request())
			.await
			.unwrap();
		let SignInStep::SignedIn(fresh) = step else {
			panic!("no second factor yet");
		};
		assert_eq!(live_sessions(&db, account).await, 1, "only the new one");
		assert!(auth.resolve(&first.token).await.unwrap().is_none());

		assert_eq!(fresh.session.account_id, account);
		auth.sign_in_with_password("hedy@example.com", BETTER, &request())
			.await
			.unwrap();
		assert!(
			events(&db, account)
				.await
				.contains(&EventKind::PasswordReset)
		);
		assert!(
			mailbox
				.latest_to("hedy@example.com")
				.unwrap()
				.subject
				.contains("reset")
		);

		// With an authenticator on, nothing changes until the second step.
		let me = auth
			.sign_in_with_password("hedy@example.com", BETTER, &request())
			.await
			.unwrap()
			.signed_in()
			.unwrap();
		let setup = auth.begin_totp(&me.signed_in()).await.unwrap();
		auth.confirm_totp(&me.signed_in(), &totp(&setup.secret, 0), &request())
			.await
			.unwrap();
		auth.send_email_code("hedy@example.com", &request())
			.await
			.unwrap();
		let emailed = code(&mailbox, "hedy@example.com");
		let SignInStep::SecondFactor(challenge) = auth
			.reset_password("hedy@example.com", &emailed, GOOD, &request())
			.await
			.unwrap()
		else {
			panic!("the reset waits for the authenticator");
		};

		assert!(matches!(
			auth.sign_in_with_password("hedy@example.com", GOOD, &request())
				.await,
			Err(AuthError::InvalidPassword)
		));
		let done = auth
			.complete_sign_in(&challenge.token, &totp(&setup.secret, 1), &request())
			.await
			.unwrap();
		assert_eq!(done.session.aal, leptoskit_auth::Aal::Two);
		assert_eq!(
			live_sessions(&db, account).await,
			1,
			"every other session ended"
		);
		assert!(matches!(
			auth.sign_in_with_password("hedy@example.com", GOOD, &request())
				.await
				.unwrap(),
			SignInStep::SecondFactor(_)
		));
	}
}

/// Changes a stored hash as an older release would have made it.
async fn store_hash(db: &Db, account: Id<Account>, hash: &str) {
	let system = db.system("test: an older hash");

	let (table, first, second) = match system.dialect() {
		Dialect::Postgres => ("auth.passwords", "$1", "$2"),
		Dialect::Sqlite => ("auth_passwords", "?1", "?2"),
	};

	system
		.execute(Statement {
			sql: format!("UPDATE {table} SET hash = {first} WHERE account_id = {second}"),
			args: vec![Value::Text(hash.to_owned()), Value::Uuid(account.uuid())],
		})
		.await
		.unwrap();
}

#[tokio::test]
async fn hashes_made_with_older_parameters_are_replaced_at_sign_in() {
	let db = leptoskit_db::testing::sqlite().await;
	let mailbox = Mailbox::new();
	let auth = auth(&db, &mailbox, Some(PasswordOptions::default()));
	let me = sign_in(&auth, &mailbox, "katherine@example.com")
		.await
		.signed_in();
	auth.set_password(&me, GOOD, &request()).await.unwrap();
	let weaker = argon2::Argon2::new(
		argon2::Algorithm::Argon2id,
		argon2::Version::V0x13,
		argon2::Params::new(4096, 1, 1, None).unwrap(),
	);
	let salt =
		argon2::password_hash::SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);

	let old = argon2::PasswordHasher::hash_password(&weaker, GOOD.as_bytes(), &salt)
		.unwrap()
		.to_string();
	store_hash(&db, me.account, &old).await;
	auth.sign_in_with_password("katherine@example.com", GOOD, &request())
		.await
		.unwrap();
	let stored = Password::query()
		.filter(|password| password.account_id.eq(me.account))
		.first(&db.system("test"))
		.await
		.unwrap()
		.unwrap();
	assert!(stored.hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
	auth.sign_in_with_password("katherine@example.com", GOOD, &request())
		.await
		.unwrap();
}

/// Have I Been Pwned, from memory: the prefixes it was asked about, and an
/// answer for each.
#[derive(Clone, Default)]
struct Pwned {
	asked: Arc<Mutex<Vec<String>>>,
	down: bool,
}

impl BreachedPasswords for Pwned {
	fn range(&self, prefix: &str) -> BreachFuture<'_> {
		self.asked.lock().unwrap().push(prefix.to_owned());
		let answer = if self.down {
			Err("unreachable".to_owned())
		} else {
			// SHA-1("correct horse battery staple") is
			// ABF7AAD6438836DBE526AA231ABDE2D0EEF74D42. Padding lines count 0.
			Ok(
				"2D6438836DBE526AA231ABDE2D0EEF74D42:0\r\nAD6438836DBE526AA231ABDE2D0EEF74D42:3\r\\
				 \
				 nF1B2C3D4E5F60718293A4B5C6D7E8F90123:0\r\n"
					.to_owned(),
			)
		};

		Box::pin(std::future::ready(answer))
	}
}

#[tokio::test]
async fn the_breach_check_sends_five_characters_of_a_hash() {
	let db = leptoskit_db::testing::sqlite().await;
	let mailbox = Mailbox::new();
	let pwned = Pwned::default();
	let auth = Auth::builder(&db, Keys::single("test", [7; 32]))
		.mailer(mailbox.clone())
		.passwords(PasswordOptions {
			breach_check: true,
			..PasswordOptions::default()
		})
		.breached_passwords(pwned.clone())
		.build()
		.unwrap();
	let me = sign_in(&auth, &mailbox, "ada@example.com")
		.await
		.signed_in();
	assert!(matches!(
		auth.set_password(&me, GOOD, &request()).await,
		Err(AuthError::WeakPassword(PasswordProblem::Breached))
	));
	assert_eq!(*pwned.asked.lock().unwrap(), ["ABF7A"]);
	auth.set_password(&me, BETTER, &request()).await.unwrap();

	// When the service is down, the password passes on the other rules.
	let down = Auth::builder(&db, Keys::single("test", [7; 32]))
		.mailer(mailbox.clone())
		.passwords(PasswordOptions {
			breach_check: true,
			..PasswordOptions::default()
		})
		.breached_passwords(Pwned {
			down: true,
			..Pwned::default()
		})
		.build()
		.unwrap();
	down.set_password(&me, GOOD, &request()).await.unwrap();
}

#[cfg(feature = "axum")]
#[tokio::test]
async fn the_routes_sign_in_with_a_cookie_and_name_what_went_wrong() {
	use axum::body::Body;
	use axum::http::Request;
	use axum::http::StatusCode;
	use axum::http::header;
	use http_body_util::BodyExt;
	use tower::ServiceExt;

	let db = leptoskit_db::testing::sqlite().await;
	let mailbox = Mailbox::new();
	let auth = Auth::builder(&db, Keys::single("test", [7; 32]))
		.origin("https://notes.example")
		.mailer(mailbox.clone())
		.passwords(PasswordOptions::default())
		.build()
		.unwrap();
	let router = axum::Router::new().merge(auth.router()).layer(auth.layer());
	let call = |method: &str, path: &str, body: serde_json::Value, cookie: Option<&str>| {
		let mut request = Request::builder()
			.method(method)
			.uri(path)
			.header(header::ORIGIN, "https://notes.example")
			.header(header::CONTENT_TYPE, "application/json");

		if let Some(cookie) = cookie {
			request = request.header(header::COOKIE, cookie);
		}

		router
			.clone()
			.oneshot(request.body(Body::from(body.to_string())).unwrap())
	};
	let json = |response: axum::response::Response| {
		async move {
			let bytes = response.into_body().collect().await.unwrap().to_bytes();
			serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default()
		}
	};

	let settings = json(
		call(
			"GET",
			"/auth/password/config",
			serde_json::Value::Null,
			None,
		)
		.await
		.unwrap(),
	)
	.await;
	assert_eq!(
		settings,
		serde_json::json!({ "enabled": true, "min_length": 15, "max_length": 256 })
	);

	// Sign up with a code, then set a password.
	auth.send_email_code("ada@example.com", &request())
		.await
		.unwrap();
	let verified = call(
		"POST",
		"/auth/email/verify",
		serde_json::json!({ "email": "ada@example.com", "code": code(&mailbox, "ada@example.com") }),
		None,
	)
	.await
	.unwrap();
	let cookie = session_cookie(&verified);
	let weak = call(
		"POST",
		"/auth/password",
		serde_json::json!({ "password": "password" }),
		Some(&cookie),
	)
	.await
	.unwrap();
	assert_eq!(weak.status(), StatusCode::BAD_REQUEST);
	assert_eq!(json(weak).await["error"], "weak_password");
	let set = call(
		"POST",
		"/auth/password",
		serde_json::json!({ "password": GOOD }),
		Some(&cookie),
	)
	.await
	.unwrap();
	assert_eq!(set.status(), StatusCode::NO_CONTENT);

	let wrong = call(
		"POST",
		"/auth/password/sign-in",
		serde_json::json!({ "email": "ada@example.com", "password": "not it, not even close" }),
		None,
	)
	.await
	.unwrap();
	assert_eq!(json(wrong).await["error"], "invalid_password");
	let signed_in = call(
		"POST",
		"/auth/password/sign-in",
		serde_json::json!({ "email": "ada@example.com", "password": GOOD }),
		None,
	)
	.await
	.unwrap();
	assert_eq!(signed_in.status(), StatusCode::OK);
	assert!(session_cookie(&signed_in).starts_with("__Host-session="));
	let bearer = json(
		call(
			"POST",
			"/auth/password/sign-in",
			serde_json::json!({ "email": "ada@example.com", "password": GOOD, "transport": "bearer" }),
			None,
		)
		.await
		.unwrap(),
	)
	.await;
	assert!(bearer["tokens"]["refresh_token"].is_string(), "{bearer}");

	auth.send_email_code("ada@example.com", &request())
		.await
		.unwrap();
	let reset = call(
		"POST",
		"/auth/password/reset",
		serde_json::json!({
			"email": "ada@example.com",
			"code": code(&mailbox, "ada@example.com"),
			"password": BETTER,
		}),
		None,
	)
	.await
	.unwrap();
	assert_eq!(reset.status(), StatusCode::OK);
	let removed = call(
		"POST",
		"/auth/password/remove",
		serde_json::Value::Null,
		Some(&cookie),
	)
	.await
	.unwrap();
	assert_eq!(
		json(removed).await["error"],
		"signed_out",
		"the reset ended that session"
	);
}

#[cfg(feature = "axum")]
fn session_cookie(response: &axum::response::Response) -> String {
	response.headers()[axum::http::header::SET_COOKIE]
		.to_str()
		.unwrap()
		.split(';')
		.next()
		.unwrap()
		.to_owned()
}
