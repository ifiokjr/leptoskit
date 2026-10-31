//! Second factors, against both databases: turning an authenticator app on
//! and off, the second step of signing in, recovery codes, and step-up.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

#[path = "../../leptoskit_db/tests/support/postgres.rs"]
mod postgres;
#[path = "support/totp.rs"]
mod totp;

use std::time::Duration;

use chrono::TimeDelta;
use leptoskit_auth::Aal;
use leptoskit_auth::Account;
use leptoskit_auth::Auth;
use leptoskit_auth::AuthError;
use leptoskit_auth::Event;
use leptoskit_auth::EventKind;
use leptoskit_auth::Factor;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::RecoveryCode;
use leptoskit_auth::RequestInfo;
use leptoskit_auth::STEP_UP_WINDOW;
use leptoskit_auth::SignIn;
use leptoskit_auth::SignInStep;
use leptoskit_auth::SignedIn;
use leptoskit_auth::TotpSecret;
use leptoskit_auth::Transport;
use leptoskit_db::Connection;
use leptoskit_db::Db;
use leptoskit_db::Dialect;
use leptoskit_db::Id;
use leptoskit_db::Statement;
use leptoskit_db::Value;
use postgres::PostgresCluster;
use totp::base32_decode;
use totp::totp;

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

fn request() -> RequestInfo {
	RequestInfo {
		ip: Some("203.0.113.9".parse().unwrap()),
		user_agent: Some("Mozilla/5.0 (Macintosh; Intel Mac OS X 14.5) Firefox/131.0".into()),
		..RequestInfo::default()
	}
}

fn auth(db: &Db, mailbox: &Mailbox) -> Auth {
	Auth::builder(db, Keys::single("test", [7; 32]))
		.app_name("Notes")
		.mailer(mailbox.clone())
		.build()
		.unwrap()
}

/// Signs `address` up or in with an emailed code, and returns the step.
async fn email(auth: &Auth, mailbox: &Mailbox, address: &str, request: &RequestInfo) -> SignInStep {
	auth.send_email_code(address, request).await.unwrap();
	let mail = mailbox.latest_to(address).unwrap();
	let code = mail.subject.rsplit(' ').next().unwrap().to_owned();
	auth.verify_email_code(address, &code, request)
		.await
		.unwrap()
}

async fn sign_in(auth: &Auth, mailbox: &Mailbox, address: &str) -> SignIn {
	email(auth, mailbox, address, &request())
		.await
		.signed_in()
		.expect("no second factor yet")
}

/// Turns an authenticator on for a fresh account, and returns its secret,
/// the elevated session, and the recovery codes.
async fn enrolled(auth: &Auth, mailbox: &Mailbox, address: &str) -> (String, SignIn, Vec<String>) {
	let sign_in = sign_in(auth, mailbox, address).await;
	let setup = auth.begin_totp(&sign_in.signed_in()).await.unwrap();
	let confirmed = auth
		.confirm_totp(&sign_in.signed_in(), &totp(&setup.secret, 0), &request())
		.await
		.unwrap();
	let codes = confirmed.recovery_codes.unwrap().codes;
	(setup.secret, confirmed.sign_in, codes)
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

/// Changes one pack row as time passing would. The pack's own writers are
/// private to it, so this goes around them: `SET {assignments} WHERE {key}
/// = $2`, with `$1` and `$2` bound to `values`.
async fn age(db: &Db, table: &str, assignments: &str, key: &str, values: [Value; 2]) {
	let system = db.system("test: age a row");
	let (table, first, second) = match system.dialect() {
		Dialect::Postgres => (format!("auth.{table}"), "$1", "$2"),
		Dialect::Sqlite => (format!("auth_{table}"), "?1", "?2"),
	};
	let assignments = assignments.replace("$1", first);
	let changed = system
		.execute(Statement {
			sql: format!("UPDATE {table} SET {assignments} WHERE {key} = {second}"),
			args: values.to_vec(),
		})
		.await
		.unwrap();
	assert_eq!(changed, 1);
}

/// Lets the authenticator's current code be used again, as the next time
/// step would.
async fn next_step(db: &Db, account: Id<Account>) {
	let step = leptoskit_db::now().timestamp().div_euclid(30) - 2;
	age(
		db,
		"totp",
		"last_used_step = $1",
		"account_id",
		[Value::Int(step), Value::Uuid(account.uuid())],
	)
	.await;
}

/// Ages the session's sign-in and last second factor past the step-up
/// window.
async fn stale(db: &Db, me: &SignedIn) -> SignedIn {
	let then = leptoskit_db::now() - TimeDelta::minutes(20);
	age(
		db,
		"sessions",
		"mfa_at = $1, authenticated_at = $1",
		"id",
		[Value::Timestamp(then), Value::Uuid(me.session.uuid())],
	)
	.await;
	SignedIn {
		mfa_at: Some(then),
		authenticated_at: then,
		..me.clone()
	}
}

#[tokio::test]
async fn an_authenticator_turns_on_with_a_matching_code() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);
		let sign_in = sign_in(&auth, &mailbox, "ada@example.com").await;
		let me = sign_in.signed_in();

		let setup = auth.begin_totp(&me).await.unwrap();
		assert_eq!(setup.secret.len(), 32, "160 bits in base32");
		assert!(
			setup
				.uri
				.starts_with("otpauth://totp/Notes:ada%40example.com?secret=")
		);
		assert!(setup.uri.contains(&setup.secret));
		assert_eq!(format!("{setup:?}"), "TotpSetup { .. }");

		// Kept encrypted: the row holds neither the secret nor its text.
		let row = TotpSecret::query()
			.filter(|row| row.account_id.eq(me.account))
			.first(&db.system("test"))
			.await
			.unwrap()
			.unwrap();
		let raw = base32_decode(&setup.secret);
		assert!(!row.secret.windows(raw.len()).any(|window| window == raw));
		assert!(row.confirmed_at.is_none());

		// Unconfirmed, it isn't a second factor yet.
		assert!(!auth.mfa_status(me.account).await.unwrap().totp);
		assert!(matches!(
			auth.confirm_totp(&me, "000000", &request()).await,
			Err(AuthError::InvalidCode)
		));

		let confirmed = auth
			.confirm_totp(&me, &totp(&setup.secret, 0), &request())
			.await
			.unwrap();
		let codes = confirmed.recovery_codes.clone().unwrap().codes;
		assert_eq!(codes.len(), 10);
		assert_eq!(
			format!("{:?}", confirmed.recovery_codes.unwrap()),
			"RecoveryCodes { count: 10, .. }"
		);

		// The session moved to aal2 with a new token; the old one is dead.
		assert_eq!(confirmed.sign_in.session.aal, Aal::Two);
		assert!(confirmed.sign_in.session.mfa_at.is_some());
		assert_eq!(confirmed.sign_in.session.id, me.session);
		assert_eq!(auth.resolve(&sign_in.token).await.unwrap(), None);
		let elevated = auth
			.resolve(&confirmed.sign_in.token)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(elevated.aal, Aal::Two);

		let status = auth.mfa_status(me.account).await.unwrap();
		assert!(status.totp);
		assert_eq!(status.recovery_codes, 10);
		assert!(matches!(
			auth.begin_totp(&elevated).await,
			Err(AuthError::AlreadyEnrolled)
		));
		assert!(matches!(
			auth.confirm_totp(&elevated, &totp(&setup.secret, 0), &request())
				.await,
			Err(AuthError::NotEnrolled)
		));

		let mail = mailbox.latest_to("ada@example.com").unwrap();
		assert_eq!(mail.subject, "Notes: Two-step verification is on");
		assert_eq!(
			events(&db, me.account).await,
			// The first code went to an address without an account.
			[
				EventKind::SignedUp,
				EventKind::SignedIn,
				EventKind::SecondFactorFailed,
				EventKind::TotpEnabled,
			]
		);
	}
}

#[tokio::test]
async fn turning_an_authenticator_on_needs_a_fresh_sign_in() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);
		let sign_in = sign_in(&auth, &mailbox, "ada@example.com").await;
		let me = stale(&db, &sign_in.signed_in()).await;

		assert!(matches!(
			auth.begin_totp(&me).await,
			Err(AuthError::Reauthenticate)
		));
		assert!(matches!(
			auth.confirm_totp(&me, "123456", &request()).await,
			Err(AuthError::Reauthenticate)
		));
	}
}

#[tokio::test]
async fn signing_in_asks_for_the_second_factor_and_refuses_a_replayed_code() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);
		let (secret, enrolled, _) = enrolled(&auth, &mailbox, "ada@example.com").await;
		let account = enrolled.session.account_id;

		let SignInStep::SecondFactor(challenge) =
			email(&auth, &mailbox, "ada@example.com", &request()).await
		else {
			panic!("the account has a second factor");
		};
		assert_eq!(challenge.factors, [Factor::Totp, Factor::RecoveryCode]);
		assert!(challenge.expires_at > leptoskit_db::now() + TimeDelta::minutes(4));
		assert_eq!(format!("{:?}", challenge.token), "SessionToken(…)");

		// The code used to turn the authenticator on works once.
		assert!(matches!(
			auth.complete_sign_in(&challenge.token, &totp(&secret, 0), &request())
				.await,
			Err(AuthError::InvalidCode)
		));
		assert!(matches!(
			auth.complete_sign_in(
				&leptoskit_auth::SessionToken::new("unknown"),
				"123456",
				&request()
			)
			.await,
			Err(AuthError::SignInExpired)
		));

		next_step(&db, account).await;
		let signed_in = auth
			.complete_sign_in(&challenge.token, &totp(&secret, 0), &request())
			.await
			.unwrap();
		assert_eq!(signed_in.session.aal, Aal::Two);
		assert!(signed_in.session.mfa_at.is_some());
		assert!(auth.resolve(&signed_in.token).await.unwrap().is_some());

		// The step is used up, and the other session survives a code.
		assert!(matches!(
			auth.complete_sign_in(&challenge.token, &totp(&secret, 0), &request())
				.await,
			Err(AuthError::SignInExpired)
		));
		assert!(auth.resolve(&enrolled.token).await.unwrap().is_some());
		assert_eq!(
			events(&db, account).await[3..],
			[
				EventKind::CodeSent,
				EventKind::SecondFactorFailed,
				EventKind::SignedIn,
			]
		);
	}
}

#[tokio::test]
async fn the_second_step_allows_five_attempts() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);
		let (secret, enrolled, _) = enrolled(&auth, &mailbox, "ada@example.com").await;
		next_step(&db, enrolled.session.account_id).await;
		let SignInStep::SecondFactor(challenge) =
			email(&auth, &mailbox, "ada@example.com", &request()).await
		else {
			panic!("the account has a second factor");
		};

		for wrong in ["000000", "111111", "222222", "333333", "444444"] {
			assert!(matches!(
				auth.complete_sign_in(&challenge.token, wrong, &request())
					.await,
				Err(AuthError::InvalidCode)
			));
		}

		// The right code no longer helps: start again.
		assert!(matches!(
			auth.complete_sign_in(&challenge.token, &totp(&secret, 0), &request())
				.await,
			Err(AuthError::SignInExpired)
		));
	}
}

#[tokio::test]
async fn a_recovery_code_signs_in_once_and_ends_the_other_sessions() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);
		let (_, enrolled, codes) = enrolled(&auth, &mailbox, "ada@example.com").await;
		let phone = RequestInfo {
			transport: Transport::Bearer,
			device_name: Some("Ada's phone".into()),
			..request()
		};
		let SignInStep::SecondFactor(challenge) =
			email(&auth, &mailbox, "ada@example.com", &phone).await
		else {
			panic!("the account has a second factor");
		};

		// Typed loosely: lower case, with spaces.
		let typed = codes[3].to_lowercase().replace('-', " ");
		let signed_in = auth
			.complete_sign_in(&challenge.token, &typed, &phone)
			.await
			.unwrap();
		assert!(signed_in.tokens().is_some(), "a native app gets tokens");
		assert_eq!(signed_in.session.device, "Ada's phone");
		assert_eq!(signed_in.session.aal, Aal::Two);

		// Every other device was signed out, and the account was told.
		assert_eq!(auth.resolve(&enrolled.token).await.unwrap(), None);
		let mail = mailbox.latest_to("ada@example.com").unwrap();
		assert_eq!(mail.subject, "Notes: A recovery code was used");
		assert_eq!(
			auth.mfa_status(enrolled.session.account_id)
				.await
				.unwrap()
				.recovery_codes,
			9
		);
		let kinds = events(&db, enrolled.session.account_id).await;
		assert!(
			kinds.ends_with(&[
				EventKind::SignedIn,
				EventKind::SessionRevoked,
				EventKind::RecoveryCodeUsed,
			]),
			"{kinds:?}"
		);

		// The code is used up.
		let SignInStep::SecondFactor(again) =
			email(&auth, &mailbox, "ada@example.com", &request()).await
		else {
			panic!("the account has a second factor");
		};
		assert!(matches!(
			auth.complete_sign_in(&again.token, &codes[3], &request())
				.await,
			Err(AuthError::InvalidCode)
		));
		auth.complete_sign_in(&again.token, &codes[4], &request())
			.await
			.unwrap();

		// Only hashes are stored.
		let stored = RecoveryCode::query()
			.filter(|row| row.account_id.eq(enrolled.session.account_id))
			.all(&db.system("test"))
			.await
			.unwrap();
		assert_eq!(stored.len(), 10);
		for code in &codes {
			let plain = code.replace('-', "");
			assert!(stored.iter().all(|row| {
				!row.code_hash
					.windows(plain.len())
					.any(|window| window == plain.as_bytes())
			}));
		}
	}
}

#[tokio::test]
async fn sensitive_changes_need_step_up_which_rotates_the_token() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);
		let (secret, enrolled, codes) = enrolled(&auth, &mailbox, "ada@example.com").await;
		let account = enrolled.session.account_id;
		let me = stale(&db, &enrolled.signed_in()).await;

		assert!(matches!(
			auth.disable_totp(&me, &request()).await,
			Err(AuthError::StepUpRequired)
		));
		assert!(matches!(
			auth.regenerate_recovery_codes(&me, &request()).await,
			Err(AuthError::StepUpRequired)
		));
		assert!(matches!(
			auth.begin_totp(&me).await,
			Err(AuthError::StepUpRequired)
		));
		assert!(matches!(
			auth.step_up(&me, "000000", &request()).await,
			Err(AuthError::InvalidCode)
		));

		next_step(&db, account).await;
		let stepped = auth
			.step_up(&me, &totp(&secret, 0), &request())
			.await
			.unwrap();
		assert_eq!(stepped.session.id, me.session);
		assert_eq!(auth.resolve(&enrolled.token).await.unwrap(), None);
		let me = auth.resolve(&stepped.token).await.unwrap().unwrap();
		assert!(me.stepped_up_within(STEP_UP_WINDOW));

		let fresh = auth
			.regenerate_recovery_codes(&me, &request())
			.await
			.unwrap();
		assert_eq!(fresh.codes.len(), 10);
		assert!(fresh.codes.iter().all(|code| !codes.contains(code)));

		// An old code no longer works anywhere.
		assert!(matches!(
			auth.step_up(&me, &codes[0], &request()).await,
			Err(AuthError::InvalidCode)
		));

		// A second device, then turning the authenticator off signs it out.
		next_step(&db, account).await;
		let SignInStep::SecondFactor(challenge) =
			email(&auth, &mailbox, "ada@example.com", &request()).await
		else {
			panic!("the account has a second factor");
		};
		let laptop = auth
			.complete_sign_in(&challenge.token, &fresh.codes[0], &request())
			.await
			.unwrap();
		// The recovery code ended this session; step up again to continue.
		assert_eq!(auth.resolve(&stepped.token).await.unwrap(), None);
		let laptop_me = laptop.signed_in();
		auth.disable_totp(&laptop_me, &request()).await.unwrap();

		let status = auth.mfa_status(account).await.unwrap();
		assert!(!status.totp);
		assert_eq!(status.recovery_codes, 0);
		assert!(matches!(
			auth.disable_totp(&laptop_me, &request()).await,
			Err(AuthError::NotEnrolled)
		));
		assert!(matches!(
			auth.regenerate_recovery_codes(&laptop_me, &request()).await,
			Err(AuthError::NotEnrolled)
		));
		assert_eq!(
			mailbox.latest_to("ada@example.com").unwrap().subject,
			"Notes: Two-step verification is off"
		);

		// Without a second factor, the email code signs straight in again.
		assert!(matches!(
			email(&auth, &mailbox, "ada@example.com", &request()).await,
			SignInStep::SignedIn(_)
		));
		let kinds = events(&db, account).await;
		assert!(kinds.contains(&EventKind::SteppedUp), "{kinds:?}");
		assert!(
			kinds.contains(&EventKind::RecoveryCodesRegenerated),
			"{kinds:?}"
		);
		assert!(kinds.contains(&EventKind::TotpDisabled), "{kinds:?}");
	}
}

#[tokio::test]
async fn second_factor_attempts_are_limited_per_account() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = Auth::builder(&db, Keys::single("test", [7; 32]))
			.mailer(mailbox.clone())
			.limits(leptoskit_auth::Limits {
				second_factor_per_account: leptoskit_auth::Limit::new(2, Duration::from_secs(600)),
				..leptoskit_auth::Limits::default()
			})
			.build()
			.unwrap();
		let (_, enrolled, _) = enrolled(&auth, &mailbox, "ada@example.com").await;
		let me = enrolled.signed_in();

		// Turning it on took one attempt.
		auth.step_up(&me, "000000", &request()).await.unwrap_err();
		assert!(matches!(
			auth.step_up(&me, "000000", &request()).await,
			Err(AuthError::RateLimited { .. })
		));
	}
}
