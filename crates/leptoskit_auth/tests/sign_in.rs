//! Signing in with email codes, against both databases: the flow, its
//! limits, the sessions it starts, and what the pack's policies let a
//! request see.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

#[path = "../../leptoskit_db/tests/support/postgres.rs"]
mod postgres;

use std::sync::Arc;
use std::time::Duration;

use leptoskit_auth::Account;
use leptoskit_auth::Auth;
use leptoskit_auth::AuthContext;
use leptoskit_auth::AuthError;
use leptoskit_auth::Challenge;
use leptoskit_auth::CodeRules;
use leptoskit_auth::DbRateLimiter;
use leptoskit_auth::EmailAddress;
use leptoskit_auth::Event;
use leptoskit_auth::EventKind;
use leptoskit_auth::Keys;
use leptoskit_auth::Limit;
use leptoskit_auth::Limits;
use leptoskit_auth::Mail;
use leptoskit_auth::MailError;
use leptoskit_auth::MailFuture;
use leptoskit_auth::Mailbox;
use leptoskit_auth::Mailer;
use leptoskit_auth::OnSignUp;
use leptoskit_auth::Profile;
use leptoskit_auth::RateLimiter;
use leptoskit_auth::RequestInfo;
use leptoskit_auth::Session;
use leptoskit_auth::SessionLifetimes;
use leptoskit_auth::SessionToken;
use leptoskit_auth::SignedIn;
use leptoskit_auth::Transport;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Tx;
use leptoskit_db::policy;
use leptoskit_db::table;
use postgres::PostgresCluster;

/// The app's own user table, sharing its key with the account.
#[table(name = "users")]
#[policy(all, |user, cx| cx.signed_in(|me| user.id.eq(me.account)))]
pub struct User {
	#[db(on_delete = cascade)]
	pub id: Id<Account>,
	#[db(max_len = 254)]
	pub name: String,
}

impl OnSignUp for User {
	async fn on_sign_up(
		tx: &Tx,
		account: Id<Account>,
		profile: &Profile,
	) -> leptoskit_auth::Result<()> {
		let email = profile
			.email
			.as_ref()
			.map(ToString::to_string)
			.unwrap_or_default();

		if email.starts_with("refused") {
			return Err(AuthError::sign_up("sign-ups from this address are closed"));
		}

		NewUser {
			id: account,
			name: email,
		}
		.insert(tx)
		.await?;
		Ok(())
	}
}

/// Both databases, each with every table, including the pack's.
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

fn keys() -> Keys {
	Keys::single("test", [7; 32])
}

fn request() -> RequestInfo {
	RequestInfo {
		ip: Some("203.0.113.9".parse().unwrap()),
		user_agent: Some("Mozilla/5.0 (Macintosh; Intel Mac OS X 14.5) Firefox/131.0".into()),
		..RequestInfo::default()
	}
}

fn auth(db: &Db, mailbox: &Mailbox) -> Auth {
	Auth::builder(db, keys())
		.app_name("Notes")
		.mailer(mailbox.clone())
		.on_sign_up::<User>()
		.build()
		.unwrap()
}

/// The code in the newest email to `address`.
fn code(mailbox: &Mailbox, address: &str) -> String {
	let mail = mailbox.latest_to(address).expect("a code was sent");
	mail.subject.rsplit(' ').next().unwrap().to_owned()
}

async fn events(db: &Db, account: Id<Account>) -> Vec<EventKind> {
	Event::query()
		.filter(|event| event.account_id.eq(account))
		.order_by(|event| event.created_at.asc())
		.all(&db.system("test: read events"))
		.await
		.unwrap()
		.into_iter()
		.map(|event| event.kind)
		.collect()
}

#[tokio::test]
async fn a_code_signs_up_then_signs_in_to_the_same_account() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);

		auth.send_email_code(" Ada@Example.com ", &request())
			.await
			.unwrap();
		let mail = mailbox.latest_to("ada@example.com").unwrap();
		assert!(mail.subject.starts_with("Your Notes sign-in code: "));
		assert!(mail.text.contains("for the next 10 minutes"));

		let first = auth
			.verify_email_code(
				"ada@example.com",
				&code(&mailbox, "ada@example.com").to_lowercase(),
				&request(),
			)
			.await
			.unwrap()
			.signed_in()
			.unwrap();
		assert!(first.signed_up);
		assert_eq!(first.session.device, "Firefox on macOS");

		let me = auth.resolve(&first.token).await.unwrap().unwrap();
		assert_eq!(me, first.signed_in());
		let user = User::get(
			&db.with_context(leptoskit_auth::context(Some(me.clone()))),
			me.account,
		)
		.await
		.unwrap();
		assert_eq!(user.name, "ada@example.com");

		auth.send_email_code("ada@example.com", &request())
			.await
			.unwrap();
		let second = auth
			.verify_email_code(
				"ADA@example.com",
				&code(&mailbox, "ada@example.com"),
				&request(),
			)
			.await
			.unwrap()
			.signed_in()
			.unwrap();
		assert!(!second.signed_up);
		assert_eq!(second.session.account_id, me.account);
		assert_ne!(second.token, first.token);

		assert_eq!(
			events(&db, me.account).await,
			[
				EventKind::SignedUp,
				EventKind::SignedIn,
				EventKind::CodeSent,
				EventKind::SignedIn
			]
		);
	}
}

#[tokio::test]
async fn a_code_works_once_and_only_the_newest_works() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);

		auth.send_email_code("ada@example.com", &request())
			.await
			.unwrap();
		let older = code(&mailbox, "ada@example.com");
		auth.send_email_code("ada@example.com", &request())
			.await
			.unwrap();
		let newer = code(&mailbox, "ada@example.com");

		let refused = auth
			.verify_email_code("ada@example.com", &older, &request())
			.await;
		assert!(
			matches!(refused, Err(AuthError::InvalidCode)),
			"{refused:?}"
		);

		auth.verify_email_code("ada@example.com", &newer, &request())
			.await
			.unwrap()
			.signed_in()
			.unwrap();
		let used_again = auth
			.verify_email_code("ada@example.com", &newer, &request())
			.await;
		assert!(
			matches!(used_again, Err(AuthError::InvalidCode)),
			"{used_again:?}"
		);

		// A code is for its own address only.
		auth.send_email_code("bob@example.com", &request())
			.await
			.unwrap();
		let elsewhere = auth
			.verify_email_code(
				"eve@example.com",
				&code(&mailbox, "bob@example.com"),
				&request(),
			)
			.await;
		assert!(
			matches!(elsewhere, Err(AuthError::InvalidCode)),
			"{elsewhere:?}"
		);
	}
}

#[tokio::test]
async fn wrong_guesses_use_up_a_code() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);

		auth.send_email_code("ada@example.com", &request())
			.await
			.unwrap();
		let right = code(&mailbox, "ada@example.com");
		let wrong = if right.starts_with('A') {
			"BBBB-BBBB"
		} else {
			"AAAA-AAAA"
		};

		for guess in [wrong, wrong, "not a code", wrong, wrong] {
			let refused = auth
				.verify_email_code("ada@example.com", guess, &request())
				.await;
			assert!(
				matches!(refused, Err(AuthError::InvalidCode)),
				"{refused:?}"
			);
		}

		let too_late = auth
			.verify_email_code("ada@example.com", &right, &request())
			.await;
		assert!(
			matches!(too_late, Err(AuthError::InvalidCode)),
			"{too_late:?}"
		);

		// The right code on the last try still works.
		auth.send_email_code("ada@example.com", &request())
			.await
			.unwrap();
		let right = code(&mailbox, "ada@example.com");

		for _ in 0..4 {
			auth.verify_email_code("ada@example.com", wrong, &request())
				.await
				.unwrap_err();
		}

		auth.verify_email_code("ada@example.com", &right, &request())
			.await
			.unwrap()
			.signed_in()
			.unwrap();
	}
}

#[tokio::test]
async fn expired_codes_and_unknown_addresses_are_refused_alike() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = Auth::builder(&db, keys())
			.mailer(mailbox.clone())
			.codes(CodeRules {
				lifetime: Duration::ZERO,
				attempts: 5,
			})
			.build()
			.unwrap();

		auth.send_email_code("ada@example.com", &request())
			.await
			.unwrap();
		assert!(
			mailbox
				.latest_to("ada@example.com")
				.unwrap()
				.text
				.contains("for the next 0 minutes")
		);
		let expired = auth
			.verify_email_code(
				"ada@example.com",
				&code(&mailbox, "ada@example.com"),
				&request(),
			)
			.await;
		assert!(
			matches!(expired, Err(AuthError::InvalidCode)),
			"{expired:?}"
		);

		let unknown = auth
			.verify_email_code("nobody@example.com", "AAAA-AAAA", &request())
			.await;
		assert!(
			matches!(unknown, Err(AuthError::InvalidCode)),
			"{unknown:?}"
		);

		for invalid in ["", "ada", "ada@example"] {
			assert!(matches!(
				auth.send_email_code(invalid, &request()).await,
				Err(AuthError::InvalidEmail)
			));
			assert!(matches!(
				auth.verify_email_code(invalid, "AAAA-AAAA", &request())
					.await,
				Err(AuthError::InvalidEmail)
			));
		}
	}
}

#[tokio::test]
async fn a_refused_sign_up_creates_nothing() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);
		let system = db.system("test: count rows");

		auth.send_email_code("refused@example.com", &request())
			.await
			.unwrap();
		let refused = auth
			.verify_email_code(
				"refused@example.com",
				&code(&mailbox, "refused@example.com"),
				&request(),
			)
			.await;
		assert!(
			matches!(&refused, Err(AuthError::SignUp(error)) if error.to_string().contains("closed")),
			"{refused:?}"
		);

		assert_eq!(Account::query().count(&system).await.unwrap(), 0);
		assert_eq!(EmailAddress::query().count(&system).await.unwrap(), 0);
		assert_eq!(Session::query().count(&system).await.unwrap(), 0);
	}
}

#[tokio::test]
async fn sessions_end_when_signed_out_revoked_rotated_or_idle() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);
		let sign_in = async |address: &str| {
			auth.send_email_code(address, &request()).await.unwrap();
			auth.verify_email_code(address, &code(&mailbox, address), &request())
				.await
				.unwrap()
				.signed_in()
				.unwrap()
		};

		let laptop = sign_in("ada@example.com").await;
		let phone = sign_in("ada@example.com").await;
		let tablet = sign_in("ada@example.com").await;
		let bob = sign_in("bob@example.com").await;
		let me = laptop.signed_in();

		// Nobody revokes someone else's session.
		assert!(!auth.revoke(&me, bob.session.id, &request()).await.unwrap());
		assert!(auth.resolve(&bob.token).await.unwrap().is_some());

		assert!(
			auth.revoke(&me, phone.session.id, &request())
				.await
				.unwrap()
		);
		assert!(
			!auth
				.revoke(&me, phone.session.id, &request())
				.await
				.unwrap()
		);
		assert_eq!(auth.resolve(&phone.token).await.unwrap(), None);

		assert_eq!(auth.revoke_others(&me, &request()).await.unwrap(), 1);
		assert_eq!(auth.revoke_others(&me, &request()).await.unwrap(), 0);
		assert_eq!(auth.resolve(&tablet.token).await.unwrap(), None);

		let rotated = auth.rotate(&me, &request()).await.unwrap();
		assert_eq!(auth.resolve(&laptop.token).await.unwrap(), None);
		let after = auth.resolve(&rotated.token).await.unwrap().unwrap();
		assert_eq!(after.authenticated_at, me.authenticated_at);
		assert!(matches!(
			auth.rotate(&me, &request()).await,
			Err(AuthError::SignedOut)
		));

		auth.sign_out(&rotated.token, &request()).await.unwrap();
		assert_eq!(auth.resolve(&rotated.token).await.unwrap(), None);
		auth.sign_out(&rotated.token, &request()).await.unwrap();
		assert_eq!(
			auth.resolve(&SessionToken::new("made-up")).await.unwrap(),
			None
		);

		let kinds = events(&db, me.account).await;
		assert_eq!(
			kinds
				.iter()
				.filter(|kind| **kind == EventKind::SessionRevoked)
				.count(),
			2
		);
		assert_eq!(kinds.last(), Some(&EventKind::SignedOut));

		// A session nobody uses ends after the idle lifetime.
		let brief = Auth::builder(&db, keys())
			.mailer(mailbox.clone())
			.sessions(SessionLifetimes {
				idle: Duration::ZERO,
				absolute: Duration::from_secs(60),
				..SessionLifetimes::default()
			})
			.build()
			.unwrap();
		brief
			.send_email_code("ada@example.com", &request())
			.await
			.unwrap();
		let idle = brief
			.verify_email_code(
				"ada@example.com",
				&code(&mailbox, "ada@example.com"),
				&request(),
			)
			.await
			.unwrap()
			.signed_in()
			.unwrap();
		assert_eq!(brief.resolve(&idle.token).await.unwrap(), None);
	}
}

#[tokio::test]
async fn native_apps_refresh_their_tokens_and_list_their_devices() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);
		let phone = RequestInfo {
			transport: Transport::Bearer,
			device_name: Some("Ada's iPhone".into()),
			..request()
		};
		auth.send_email_code("ada@example.com", &phone)
			.await
			.unwrap();
		let first = auth
			.verify_email_code(
				"ada@example.com",
				&code(&mailbox, "ada@example.com"),
				&phone,
			)
			.await
			.unwrap()
			.signed_in()
			.unwrap();
		let me = auth.resolve(&first.token).await.unwrap().unwrap();

		let second = auth
			.refresh(first.refresh.as_ref().unwrap(), &phone)
			.await
			.unwrap();
		assert_eq!(auth.resolve(&first.token).await.unwrap(), None);
		// A retry of a refresh whose response was lost still works.
		let retried = auth
			.refresh(first.refresh.as_ref().unwrap(), &phone)
			.await
			.unwrap();
		assert!(matches!(
			auth.refresh(second.refresh.as_ref().unwrap(), &phone).await,
			Err(AuthError::SignedOut)
		));
		assert!(auth.resolve(&retried.token).await.unwrap().is_some());

		// A browser signs in too; each sees both devices.
		auth.send_email_code("ada@example.com", &request())
			.await
			.unwrap();
		let laptop = auth
			.verify_email_code(
				"ada@example.com",
				&code(&mailbox, "ada@example.com"),
				&request(),
			)
			.await
			.unwrap()
			.signed_in()
			.unwrap();
		let devices = auth.devices(&me).await.unwrap();
		let mut seen: Vec<_> = devices
			.iter()
			.map(|device| (device.session.device.as_str(), device.current))
			.collect();
		seen.sort_unstable();
		assert_eq!(seen, [("Ada's iPhone", true), ("Firefox on macOS", false)]);

		// An access token that has already expired still refreshes.
		let instant = Auth::builder(&db, keys())
			.mailer(mailbox.clone())
			.sessions(SessionLifetimes {
				access: Duration::ZERO,
				..SessionLifetimes::default()
			})
			.build()
			.unwrap();
		let expired = instant
			.refresh(retried.refresh.as_ref().unwrap(), &phone)
			.await
			.unwrap();
		assert_eq!(instant.resolve(&expired.token).await.unwrap(), None);
		assert_eq!(expired.tokens().unwrap().expires_in, 0);

		assert_eq!(
			auth.sign_out_everywhere(me.account, &request())
				.await
				.unwrap(),
			2
		);
		assert_eq!(auth.resolve(&laptop.token).await.unwrap(), None);
		assert!(matches!(
			auth.refresh(expired.refresh.as_ref().unwrap(), &phone)
				.await,
			Err(AuthError::SignedOut)
		));
		assert_eq!(auth.devices(&me).await.unwrap(), Vec::new());
	}
}

#[tokio::test]
async fn policies_show_each_person_only_their_own_rows() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);
		let mut people: Vec<SignedIn> = Vec::new();

		for address in ["ada@example.com", "bob@example.com"] {
			auth.send_email_code(address, &request()).await.unwrap();
			let sign_in = auth
				.verify_email_code(address, &code(&mailbox, address), &request())
				.await
				.unwrap()
				.signed_in()
				.unwrap();
			people.push(sign_in.signed_in());
		}

		let ada = db.with_context(leptoskit_auth::context(Some(people[0].clone())));
		let anonymous = db.with_context(leptoskit_auth::context(None));

		let sessions = Session::query().all(&ada).await.unwrap();
		assert_eq!(sessions.len(), 1);
		assert_eq!(sessions[0].account_id, people[0].account);
		assert_eq!(Account::query().count(&ada).await.unwrap(), 1);
		assert_eq!(EmailAddress::query().count(&ada).await.unwrap(), 1);
		assert_eq!(User::query().count(&ada).await.unwrap(), 1);
		assert!(Event::query().count(&ada).await.unwrap() > 0);

		// Codes and counters are for the pack alone.
		assert_eq!(Challenge::query().count(&ada).await.unwrap(), 0);
		assert!(Challenge::query().count(&db.system("test")).await.unwrap() > 0);

		for table_count in [
			Session::query().count(&anonymous).await.unwrap(),
			Account::query().count(&anonymous).await.unwrap(),
			User::query().count(&anonymous).await.unwrap(),
		] {
			assert_eq!(table_count, 0);
		}

		let cx = leptoskit_auth::context(Some(people[1].clone()));
		assert_eq!(cx.user_id::<User>(), Some(people[1].user_id()));
		assert_eq!(cx.actor(), Some(leptoskit_db::Actor::of(people[1].account)));
		assert_eq!(leptoskit_auth::context(None).user_id::<User>(), None);
		assert_eq!(leptoskit_auth::context(None).actor(), None);
	}
}

#[tokio::test]
async fn too_many_attempts_are_rate_limited() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = Auth::builder(&db, keys())
			.mailer(mailbox.clone())
			.limits(Limits {
				code_per_address: Limit::new(2, Duration::from_secs(600)),
				code_per_ip: Limit::new(4, Duration::from_secs(600)),
				verify_per_ip: Limit::new(2, Duration::from_secs(600)),
				..Limits::default()
			})
			.build()
			.unwrap();

		auth.send_email_code("ada@example.com", &request())
			.await
			.unwrap();
		auth.send_email_code("ada@example.com", &request())
			.await
			.unwrap();
		let limited = auth.send_email_code("ada@example.com", &request()).await;
		assert!(
			matches!(limited, Err(AuthError::RateLimited { retry_after }) if retry_after > Duration::from_secs(500)),
			"{limited:?}"
		);

		// The IP's own limit covers every address it asks for, and counts
		// the refused request too.
		auth.send_email_code("bob@example.com", &request())
			.await
			.unwrap();
		assert!(matches!(
			auth.send_email_code("eve@example.com", &request()).await,
			Err(AuthError::RateLimited { .. })
		));
		let elsewhere = RequestInfo {
			ip: Some("198.51.100.4".parse().unwrap()),
			..request()
		};
		auth.send_email_code("eve@example.com", &elsewhere)
			.await
			.unwrap();

		for _ in 0..2 {
			auth.verify_email_code("ada@example.com", "AAAA-AAAA", &request())
				.await
				.unwrap_err();
		}

		assert!(matches!(
			auth.verify_email_code("ada@example.com", "AAAA-AAAA", &request())
				.await,
			Err(AuthError::RateLimited { .. })
		));
	}
}

#[tokio::test]
async fn a_window_resets_and_concurrent_attempts_share_it() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let limiter = Arc::new(DbRateLimiter::new(&db));
		let instant = Limit::new(1, Duration::ZERO);

		// A window that has passed starts again.
		limiter.hit(b"reset", instant).await.unwrap();
		limiter.hit(b"reset", instant).await.unwrap();

		let limit = Limit::new(5, Duration::from_secs(600));
		let attempts = (0..12).map(|_| {
			let limiter = Arc::clone(&limiter);
			tokio::spawn(async move { limiter.hit(b"busy", limit).await })
		});
		let mut allowed = 0;

		for attempt in attempts {
			match attempt.await.unwrap() {
				Ok(()) => allowed += 1,
				Err(AuthError::RateLimited { .. }) => {}
				Err(error) => panic!("{error}"),
			}
		}

		assert!(allowed <= 5, "{allowed} attempts got through a limit of 5");
		assert!(allowed >= 1);
	}
}

/// A mailer that's down.
struct Broken;

impl Mailer for Broken {
	fn send(&self, _mail: Mail) -> MailFuture<'_> {
		Box::pin(async { Err(MailError("the provider is down".into())) })
	}
}

#[tokio::test]
async fn a_mailer_failure_is_reported() {
	let db = databases(None).await.remove(0);
	let auth = Auth::builder(&db, keys()).mailer(Broken).build().unwrap();

	let failed = auth.send_email_code("ada@example.com", &request()).await;
	assert_eq!(
		failed.unwrap_err().to_string(),
		"the email couldn't be sent: the provider is down"
	);

	let missing = Auth::builder(&db, keys()).build().unwrap_err();
	assert!(missing.to_string().contains("set a mailer"));
	assert!(format!("{auth:?}").contains(r#"keys: Keys { ids: ["test"], .. }"#));
}

#[tokio::test]
async fn a_disabled_account_is_signed_out_everywhere() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);

		auth.send_email_code("ada@example.com", &request())
			.await
			.unwrap();
		let sign_in = auth
			.verify_email_code(
				"ada@example.com",
				&code(&mailbox, "ada@example.com"),
				&request(),
			)
			.await
			.unwrap()
			.signed_in()
			.unwrap();
		let account = sign_in.session.account_id;

		auth.disable(account, &request()).await.unwrap();
		assert_eq!(auth.resolve(&sign_in.token).await.unwrap(), None);
		auth.send_email_code("ada@example.com", &request())
			.await
			.unwrap();
		let refused = auth
			.verify_email_code(
				"ada@example.com",
				&code(&mailbox, "ada@example.com"),
				&request(),
			)
			.await;
		assert!(matches!(refused, Err(AuthError::Disabled)), "{refused:?}");

		auth.enable(account).await.unwrap();
		assert_eq!(auth.resolve(&sign_in.token).await.unwrap(), None);
		auth.send_email_code("ada@example.com", &request())
			.await
			.unwrap();
		let back = auth
			.verify_email_code(
				"ada@example.com",
				&code(&mailbox, "ada@example.com"),
				&request(),
			)
			.await
			.unwrap()
			.signed_in()
			.unwrap();
		assert_eq!(back.session.account_id, account);
	}
}

/// A limiter that refuses everything.
struct Closed;

impl RateLimiter for Closed {
	fn hit<'a>(&'a self, _key: &'a [u8], _limit: Limit) -> leptoskit_auth::LimitFuture<'a> {
		Box::pin(async {
			Err(AuthError::RateLimited {
				retry_after: Duration::from_secs(30),
			})
		})
	}
}

#[tokio::test]
async fn apps_choose_the_limiter_and_may_skip_the_sign_up_hook() {
	let db = databases(None).await.remove(0);
	let mailbox = Mailbox::new();
	let closed = Auth::builder(&db, keys())
		.mailer(mailbox.clone())
		.rate_limiter(Closed)
		.build()
		.unwrap();
	assert_eq!(closed.app_name(), "this app");
	assert_eq!(
		closed
			.send_email_code("ada@example.com", &request())
			.await
			.unwrap_err()
			.to_string(),
		"too many attempts; try again in 30 seconds"
	);
	assert_eq!(mailbox.messages(), [] as [Mail; 0]);

	// Without a hook, an account is all a sign-up creates.
	let open = Auth::builder(&db, keys())
		.mailer(mailbox.clone())
		.build()
		.unwrap();
	open.send_email_code("ada@example.com", &request())
		.await
		.unwrap();
	assert_eq!(mailbox.messages().len(), 1);
	let sign_in = open
		.verify_email_code(
			"ada@example.com",
			&code(&mailbox, "ada@example.com"),
			&request(),
		)
		.await
		.unwrap()
		.signed_in()
		.unwrap();
	assert!(sign_in.signed_up);
	assert_eq!(User::query().count(&db.system("test")).await.unwrap(), 0);
}
