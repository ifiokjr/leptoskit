//! Passkeys, against both databases: adding them, signing in with them on
//! their own and as the second step, step-up, management, and what a
//! required second factor allows.

#![cfg(all(feature = "postgres", feature = "sqlite", feature = "passkey"))]

#[path = "support/passkey.rs"]
mod passkey;
#[path = "../../leptoskit_db/tests/support/postgres.rs"]
mod postgres;

use chrono::TimeDelta;
use leptoskit_auth::Aal;
use leptoskit_auth::Account;
use leptoskit_auth::Auth;
use leptoskit_auth::AuthError;
use leptoskit_auth::EmailAddress;
use leptoskit_auth::Event;
use leptoskit_auth::EventKind;
use leptoskit_auth::Factor;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::MfaPolicy;
use leptoskit_auth::Passkey;
use leptoskit_auth::PasskeyInfo;
use leptoskit_auth::RecoveryCode;
use leptoskit_auth::RequestInfo;
use leptoskit_auth::Roles;
use leptoskit_auth::SignIn;
use leptoskit_auth::SignInStep;
use leptoskit_auth::SignedIn;
use leptoskit_db::Connection;
use leptoskit_db::Db;
use leptoskit_db::Dialect;
use leptoskit_db::Id;
use leptoskit_db::Statement;
use leptoskit_db::Value;
use passkey::SoftPasskey;
use postgres::PostgresCluster;

const ORIGIN: &str = "https://notes.example";

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
	builder(db, mailbox).build().unwrap()
}

fn builder(db: &Db, mailbox: &Mailbox) -> leptoskit_auth::AuthBuilder {
	Auth::builder(db, Keys::single("test", [7; 32]))
		.app_name("Notes")
		.origin(ORIGIN)
		.mailer(mailbox.clone())
}

async fn email(auth: &Auth, mailbox: &Mailbox, address: &str) -> SignInStep {
	auth.send_email_code(address, &request()).await.unwrap();
	let mail = mailbox.latest_to(address).unwrap();
	let code = mail.subject.rsplit(' ').next().unwrap().to_owned();
	auth.verify_email_code(address, &code, &request())
		.await
		.unwrap()
}

async fn sign_in(auth: &Auth, mailbox: &Mailbox, address: &str) -> SignIn {
	email(auth, mailbox, address)
		.await
		.signed_in()
		.expect("no second factor yet")
}

/// Signs `address` up and adds a passkey on `device`, returning the
/// elevated session and the recovery codes.
async fn with_passkey(
	auth: &Auth,
	mailbox: &Mailbox,
	address: &str,
	device: &mut SoftPasskey,
) -> (SignIn, Vec<String>) {
	let me = sign_in(auth, mailbox, address).await.signed_in();
	let options = auth.begin_passkey_registration(&me).await.unwrap();
	let added = auth
		.finish_passkey_registration(&me, &device.register(&options), None, &request())
		.await
		.unwrap();
	(added.sign_in, added.recovery_codes.unwrap().codes)
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

/// Ages the session's sign-in and last second factor past the step-up
/// window.
async fn stale(db: &Db, me: &SignedIn) -> SignedIn {
	let then = leptoskit_db::now() - TimeDelta::minutes(20);
	let system = db.system("test: age a session");
	let (table, first, second) = match system.dialect() {
		Dialect::Postgres => ("auth.sessions", "$1", "$2"),
		Dialect::Sqlite => ("auth_sessions", "?1", "?2"),
	};
	system
		.execute(Statement {
			sql: format!(
				"UPDATE {table} SET mfa_at = {first}, authenticated_at = {first} WHERE id = \
				 {second}"
			),
			args: vec![Value::Timestamp(then), Value::Uuid(me.session.uuid())],
		})
		.await
		.unwrap();
	SignedIn {
		mfa_at: Some(then),
		authenticated_at: then,
		..me.clone()
	}
}

#[tokio::test]
async fn a_passkey_is_added_and_then_signs_in_on_its_own() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);
		let mut device = SoftPasskey::new(ORIGIN);
		device.synced = true;
		let me = sign_in(&auth, &mailbox, "ada@example.com")
			.await
			.signed_in();

		let options = auth.begin_passkey_registration(&me).await.unwrap();
		let json = serde_json::to_value(&options).unwrap();
		let selection = &json["publicKey"]["authenticatorSelection"];
		assert_eq!(selection["residentKey"], "required");
		assert_eq!(selection["userVerification"], "required");
		assert_eq!(json["publicKey"]["rp"]["id"], "notes.example");

		let credential = device.register(&options);
		let added = auth
			.finish_passkey_registration(&me, &credential, Some("  Laptop  "), &request())
			.await
			.unwrap();
		assert_eq!(added.passkey.name, "Laptop");
		assert!(added.passkey.backed_up);
		assert_eq!(added.recovery_codes.as_ref().unwrap().codes.len(), 10);
		// Like confirming an authenticator: `aal2`, with a new token.
		assert_eq!(added.sign_in.session.aal, Aal::Two);
		assert_eq!(
			auth.resolve(&added.sign_in.token)
				.await
				.unwrap()
				.unwrap()
				.account,
			me.account
		);
		assert!(
			mailbox
				.latest_to("ada@example.com")
				.unwrap()
				.subject
				.contains("A passkey was added")
		);

		// The registration ceremony worked once; and now that the account has
		// a factor, changing them needs step-up, which the old view of the
		// session hasn't done.
		let elevated = added.sign_in.signed_in();
		assert!(matches!(
			auth.finish_passkey_registration(&elevated, &credential, None, &request())
				.await,
			Err(AuthError::PasskeyFailed)
		));
		assert!(matches!(
			auth.finish_passkey_registration(&me, &credential, None, &request())
				.await,
			Err(AuthError::StepUpRequired)
		));

		let status = auth.mfa_status(me.account).await.unwrap();
		assert_eq!(status.passkeys, 1);
		assert!(status.enrolled());
		assert!(!status.required);

		// Signing in with it: no username, no email, `aal2` at once.
		let challenge = auth.begin_passkey_sign_in(&request()).await.unwrap();
		let options = serde_json::to_value(&challenge.options).unwrap();
		assert_eq!(options["mediation"], "conditional");
		assert!(
			options["publicKey"]["allowCredentials"]
				.as_array()
				.is_none_or(Vec::is_empty)
		);
		let assertion = device.sign(&challenge.options);
		let signed_in = auth
			.finish_passkey_sign_in(&challenge.token, &assertion, &request())
			.await
			.unwrap();
		assert_eq!(signed_in.session.account_id, me.account);
		assert_eq!(signed_in.session.aal, Aal::Two);
		assert!(
			signed_in
				.signed_in()
				.stepped_up_within(leptoskit_auth::STEP_UP_WINDOW)
		);

		// Each ceremony works once.
		assert!(matches!(
			auth.finish_passkey_sign_in(&challenge.token, &assertion, &request())
				.await,
			Err(AuthError::PasskeyFailed)
		));

		let passkeys = auth.passkeys(me.account).await.unwrap();
		assert_eq!(passkeys.len(), 1);
		assert!(passkeys[0].last_used_at.is_some());
		let kinds = events(&db, me.account).await;
		assert!(kinds.contains(&EventKind::PasskeyAdded));
		assert_eq!(kinds.last(), Some(&EventKind::SignedIn));

		// Nothing secret is stored: the row holds a public key.
		let row = Passkey::query()
			.filter(|row| row.account_id.eq(me.account))
			.first(&db.system("test"))
			.await
			.unwrap()
			.unwrap();
		let stored = String::from_utf8(row.credential).unwrap();
		assert!(stored.contains("\"counter\":1"), "{stored}");
	}
}

#[tokio::test]
async fn passkeys_from_other_sites_cloned_keys_and_strangers_are_refused() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);
		let mut device = SoftPasskey::new(ORIGIN);
		let (sign_in, _) = with_passkey(&auth, &mailbox, "ada@example.com", &mut device).await;
		let account = sign_in.session.account_id;

		// A phishing page on another origin gets a signature the server
		// refuses.
		let mut phished = SoftPasskey::new(ORIGIN);
		std::mem::swap(&mut phished, &mut device);
		let mut phished = phished.on("https://notes.example.evil");
		let challenge = auth.begin_passkey_sign_in(&request()).await.unwrap();
		let assertion = phished.sign(&challenge.options);
		assert!(matches!(
			auth.finish_passkey_sign_in(&challenge.token, &assertion, &request())
				.await,
			Err(AuthError::PasskeyFailed)
		));
		assert_eq!(
			events(&db, account).await.last(),
			Some(&EventKind::PasskeyFailed)
		);
		let mut device = phished.on(ORIGIN);

		// A good sign-in moves the counter on; a copy of the key that fell
		// behind is refused.
		let challenge = auth.begin_passkey_sign_in(&request()).await.unwrap();
		let assertion = device.sign(&challenge.options);
		auth.finish_passkey_sign_in(&challenge.token, &assertion, &request())
			.await
			.unwrap();
		device.set_counter(0, 0);
		let challenge = auth.begin_passkey_sign_in(&request()).await.unwrap();
		let assertion = device.sign(&challenge.options);
		assert!(matches!(
			auth.finish_passkey_sign_in(&challenge.token, &assertion, &request())
				.await,
			Err(AuthError::PasskeyFailed)
		));

		// A passkey nobody registered, and a token for no ceremony.
		let mut stranger = SoftPasskey::new(ORIGIN);
		let other = sign_in_elsewhere(&db).await;
		let options = Auth::builder(&db, Keys::single("test", [7; 32]))
			.origin(ORIGIN)
			.mailer(Mailbox::new())
			.build()
			.unwrap()
			.begin_passkey_registration(&other)
			.await
			.unwrap();
		stranger.register(&options);
		let challenge = auth.begin_passkey_sign_in(&request()).await.unwrap();
		let assertion = stranger.sign(&challenge.options);
		assert!(matches!(
			auth.finish_passkey_sign_in(&challenge.token, &assertion, &request())
				.await,
			Err(AuthError::PasskeyFailed)
		));
		assert!(matches!(
			auth.finish_passkey_sign_in(
				&leptoskit_auth::SessionToken::new("no such ceremony"),
				&assertion,
				&request()
			)
			.await,
			Err(AuthError::PasskeyFailed)
		));

		// A disabled account's passkey signs no one in.
		auth.disable(account, &request()).await.unwrap();
		let challenge = auth.begin_passkey_sign_in(&request()).await.unwrap();
		let assertion = device.sign(&challenge.options);
		assert!(matches!(
			auth.finish_passkey_sign_in(&challenge.token, &assertion, &request())
				.await,
			Err(AuthError::Disabled)
		));
	}
}

/// A fresh session for a new account, made without email.
async fn sign_in_elsewhere(db: &Db) -> SignedIn {
	let mailbox = Mailbox::new();
	let auth = auth(db, &mailbox);
	sign_in(&auth, &mailbox, "grace@example.com")
		.await
		.signed_in()
}

#[tokio::test]
async fn an_emailed_code_then_asks_for_the_passkey() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);
		let mut device = SoftPasskey::new(ORIGIN);
		let (_, codes) = with_passkey(&auth, &mailbox, "ada@example.com", &mut device).await;

		let SignInStep::SecondFactor(step) = email(&auth, &mailbox, "ada@example.com").await else {
			panic!("the passkey is a second factor now");
		};
		assert_eq!(step.factors, [Factor::Passkey, Factor::RecoveryCode]);

		// Without a ceremony, or answering another one, it fails and counts.
		let options = auth
			.begin_passkey_second_factor(&step.token, &request())
			.await
			.unwrap();
		let allowed = serde_json::to_value(&options).unwrap()["publicKey"]["allowCredentials"]
			.as_array()
			.unwrap()
			.len();
		assert_eq!(allowed, 1, "only the account's own passkeys");
		let assertion = device.sign(&options);
		let signed_in = auth
			.complete_sign_in_with_passkey(&step.token, &assertion, &request())
			.await
			.unwrap();
		assert_eq!(signed_in.session.aal, Aal::Two);

		// The step is over.
		assert!(matches!(
			auth.begin_passkey_second_factor(&step.token, &request())
				.await,
			Err(AuthError::SignInExpired)
		));

		// A wrong answer leaves the step to try again, and a recovery code
		// still works.
		let SignInStep::SecondFactor(step) = email(&auth, &mailbox, "ada@example.com").await else {
			panic!("a second factor");
		};
		let options = auth
			.begin_passkey_second_factor(&step.token, &request())
			.await
			.unwrap();
		let assertion = device.on("https://elsewhere.example").sign(&options);
		assert!(matches!(
			auth.complete_sign_in_with_passkey(&step.token, &assertion, &request())
				.await,
			Err(AuthError::PasskeyFailed)
		));
		let account = signed_in.session.account_id;
		assert_eq!(
			events(&db, account).await.last(),
			Some(&EventKind::SecondFactorFailed)
		);
		let finished = auth
			.complete_sign_in(&step.token, &codes[0], &request())
			.await
			.unwrap();
		assert_eq!(finished.session.aal, Aal::Two);
	}
}

#[tokio::test]
async fn passkeys_step_up_and_are_managed_after_it() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = auth(&db, &mailbox);
		let mut device = SoftPasskey::new(ORIGIN);
		let (sign_in, _) = with_passkey(&auth, &mailbox, "ada@example.com", &mut device).await;
		let me = stale(&db, &sign_in.signed_in()).await;
		let id = auth.passkeys(me.account).await.unwrap()[0].id;

		// Renaming is plain; removing needs step-up.
		auth.rename_passkey(&me, id, " Phone ").await.unwrap();
		assert_eq!(auth.passkeys(me.account).await.unwrap()[0].name, "Phone");
		assert!(matches!(
			auth.rename_passkey(&me, id, "   ").await,
			Err(AuthError::InvalidName)
		));
		assert!(matches!(
			auth.rename_passkey(&me, Id::new(), "Phone").await,
			Err(AuthError::NotFound)
		));
		assert!(matches!(
			auth.remove_passkey(&me, id, &request()).await,
			Err(AuthError::StepUpRequired)
		));

		let options = auth.begin_passkey_step_up(&me, &request()).await.unwrap();
		let stepped = auth
			.step_up_with_passkey(&me, &device.sign(&options), &request())
			.await
			.unwrap();
		let me = stepped.signed_in();
		assert!(me.stepped_up_within(leptoskit_auth::STEP_UP_WINDOW));
		// A replayed answer to the same ceremony is refused, and counted.
		let options = auth.begin_passkey_step_up(&me, &request()).await.unwrap();
		let assertion = device.sign(&options);
		auth.step_up_with_passkey(&me, &assertion, &request())
			.await
			.unwrap();
		assert!(matches!(
			auth.step_up_with_passkey(&me, &assertion, &request()).await,
			Err(AuthError::PasskeyFailed)
		));

		// The only passkey of an account with email still goes; the
		// recovery codes go with the last factor, and other devices sign
		// out.
		let elsewhere = sign_in_with(&auth, &mut device).await;
		auth.remove_passkey(&me, id, &request()).await.unwrap();
		assert!(auth.resolve(&elsewhere.token).await.unwrap().is_none());
		assert_eq!(
			auth.passkeys(me.account).await.unwrap(),
			[] as [PasskeyInfo; 0]
		);
		let codes = RecoveryCode::query()
			.filter(|code| code.account_id.eq(me.account))
			.count(&db.system("test"))
			.await
			.unwrap();
		assert_eq!(codes, 0);
		assert!(matches!(
			auth.remove_passkey(&me, id, &request()).await,
			Err(AuthError::NotFound)
		));
		assert!(
			events(&db, me.account)
				.await
				.contains(&EventKind::PasskeyRemoved)
		);
		assert!(matches!(
			auth.begin_passkey_step_up(&me, &request()).await,
			Err(AuthError::NotEnrolled)
		));
	}
}

async fn sign_in_with(auth: &Auth, device: &mut SoftPasskey) -> SignIn {
	let challenge = auth.begin_passkey_sign_in(&request()).await.unwrap();
	let assertion = device.sign(&challenge.options);
	auth.finish_passkey_sign_in(&challenge.token, &assertion, &request())
		.await
		.unwrap()
}

/// Admins are anyone whose address starts with `admin`.
struct Staff;

impl Roles for Staff {
	async fn roles(db: &Db, account: Id<Account>) -> leptoskit_auth::Result<Vec<String>> {
		let emails = EmailAddress::query()
			.filter(|email| email.account_id.eq(account))
			.all(db)
			.await?;
		Ok(emails
			.iter()
			.filter(|email| email.address.as_str().starts_with("admin"))
			.map(|_| "Admin".to_owned())
			.collect())
	}
}

#[tokio::test]
async fn a_required_second_factor_limits_sessions_to_setting_one_up() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let mailbox = Mailbox::new();
		let auth = builder(&db, &mailbox)
			.mfa(MfaPolicy::required_for(["Admin"]))
			.roles::<Staff>()
			.build()
			.unwrap();

		// Someone without the role is unaffected.
		let member = sign_in(&auth, &mailbox, "ada@example.com").await;
		assert!(auth.resolve(&member.token).await.unwrap().is_some());
		assert!(
			!auth
				.mfa_status(member.session.account_id)
				.await
				.unwrap()
				.required
		);

		// An admin's first session may only set a factor up.
		let admin = sign_in(&auth, &mailbox, "admin@example.com").await;
		let me = admin.signed_in();
		assert_eq!(auth.resolve(&admin.token).await.unwrap(), None);
		let status = auth.mfa_status(me.account).await.unwrap();
		assert!(status.required);
		assert!(!status.enrolled());

		let mut device = SoftPasskey::new(ORIGIN);
		let options = auth.begin_passkey_registration(&me).await.unwrap();
		let added = auth
			.finish_passkey_registration(&me, &device.register(&options), None, &request())
			.await
			.unwrap();
		assert!(auth.resolve(&added.sign_in.token).await.unwrap().is_some());

		// The required factor can't be removed while it's the only one.
		let me = added.sign_in.signed_in();
		assert!(matches!(
			auth.remove_passkey(&me, added.passkey.id, &request()).await,
			Err(AuthError::LastSignInMethod)
		));

		// Everyone, with `required`.
		let strict = builder(&db, &mailbox)
			.mfa(MfaPolicy::required())
			.build()
			.unwrap();
		assert!(strict.resolve(&member.token).await.unwrap().is_none());
		assert!(
			strict
				.mfa_status(member.session.account_id)
				.await
				.unwrap()
				.required
		);

		// Roles without a way to read them is a configuration error.
		assert!(matches!(
			builder(&db, &mailbox)
				.mfa(MfaPolicy::required_for(["Admin"]))
				.build(),
			Err(AuthError::Config(_))
		));
	}
}

#[tokio::test]
async fn passkeys_need_an_origin_and_a_fresh_session() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	let mailbox = Mailbox::new();
	let no_origin = Auth::builder(&db, Keys::single("test", [7; 32]))
		.mailer(mailbox.clone())
		.build()
		.unwrap();
	let me = sign_in(&no_origin, &mailbox, "ada@example.com")
		.await
		.signed_in();
	assert!(matches!(
		no_origin.begin_passkey_registration(&me).await,
		Err(AuthError::Config(message)) if message.contains("[auth] origin")
	));
	assert!(matches!(
		Auth::builder(&db, Keys::single("test", [7; 32]))
			.origin("not a url")
			.mailer(mailbox.clone())
			.build(),
		Err(AuthError::Config(message)) if message.contains("relying party")
	));

	let auth = auth(&db, &mailbox);
	let me = stale(&db, &me).await;
	assert!(matches!(
		auth.begin_passkey_registration(&me).await,
		Err(AuthError::Reauthenticate)
	));
}

#[tokio::test]
async fn rotating_keys_re_encrypts_ceremonies_in_flight() {
	use base64::Engine;

	let encode = |byte: u8| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([byte; 32]);
	let db = Db::connect("sqlite::memory:").await.unwrap();
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	let mailbox = Mailbox::new();
	let with = |keys: &str| {
		Auth::builder(&db, Keys::parse(keys).unwrap())
			.origin(ORIGIN)
			.mailer(mailbox.clone())
			.build()
			.unwrap()
	};
	let old = with(&format!("old:{}", encode(1)));
	let mut device = SoftPasskey::new(ORIGIN);
	let (..) = with_passkey(&old, &mailbox, "ada@example.com", &mut device).await;
	let challenge = old.begin_passkey_sign_in(&request()).await.unwrap();

	let both = with(&format!("new:{},old:{}", encode(2), encode(1)));
	let rotated = both.rotate_keys().await.unwrap();
	assert_eq!(rotated.ceremonies, 1);
	assert_eq!(rotated.totp, 0);

	// The old key can go; the sign-in in flight still finishes.
	let new = with(&format!("new:{}", encode(2)));
	let assertion = device.sign(&challenge.options);
	new.finish_passkey_sign_in(&challenge.token, &assertion, &request())
		.await
		.unwrap();
}

#[tokio::test]
async fn the_native_apps_passkeys_count_as_the_sites() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	let mailbox = Mailbox::new();
	let signing = [5; 32];
	let android = format!(
		"android:apk-key-hash:{}",
		base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, signing)
	);
	let auth = builder(&db, &mailbox)
		.native_apps(leptoskit_auth::NativeApps {
			package_id: "com.notes.app".into(),
			apple_team_id: Some("ABCDE12345".into()),
			android_certificates: vec![signing],
		})
		.build()
		.unwrap();
	assert_eq!(
		leptoskit_auth::NativeApps {
			android_certificates: vec![signing],
			..leptoskit_auth::NativeApps::default()
		}
		.android_origins(),
		std::slice::from_ref(&android)
	);

	// Made in the Android app, which reports its signing certificate as the
	// origin, then used there and on the web.
	let mut device = SoftPasskey::new(&android);
	with_passkey(&auth, &mailbox, "ada@example.com", &mut device).await;
	let challenge = auth.begin_passkey_sign_in(&request()).await.unwrap();
	let assertion = device.sign(&challenge.options);
	auth.finish_passkey_sign_in(&challenge.token, &assertion, &request())
		.await
		.unwrap();
	let mut device = device.on(ORIGIN);
	let challenge = auth.begin_passkey_sign_in(&request()).await.unwrap();
	let assertion = device.sign(&challenge.options);
	auth.finish_passkey_sign_in(&challenge.token, &assertion, &request())
		.await
		.unwrap();

	// An app signed with another certificate is someone else's.
	let mut impostor = device.on(&format!(
		"android:apk-key-hash:{}",
		base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, [6; 32])
	));
	let challenge = auth.begin_passkey_sign_in(&request()).await.unwrap();
	let assertion = impostor.sign(&challenge.options);
	assert!(matches!(
		auth.finish_passkey_sign_in(&challenge.token, &assertion, &request())
			.await,
		Err(AuthError::PasskeyFailed)
	));

	// Without the app configured, its certificate's origin is refused too.
	let plain = builder(&db, &mailbox).build().unwrap();
	let mut device = impostor.on(&android);
	let challenge = plain.begin_passkey_sign_in(&request()).await.unwrap();
	let assertion = device.sign(&challenge.options);
	assert!(matches!(
		plain
			.finish_passkey_sign_in(&challenge.token, &assertion, &request())
			.await,
		Err(AuthError::PasskeyFailed)
	));
}
