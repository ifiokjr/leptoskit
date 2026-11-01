//! Solana wallets through the real routes, with a software wallet whose key
//! is made at test time: linking with a fresh Sign-In With Solana proof,
//! replayed and forged answers, opt-in sign-in, fresh proofs, unlinking,
//! `cx.wallets()` in policies, and the merge offer a wallet can make.

#![cfg(all(
	feature = "postgres",
	feature = "sqlite",
	feature = "solana",
	feature = "axum"
))]

#[path = "../../leptoskit_db/tests/support/postgres.rs"]
mod postgres;
#[path = "support/totp.rs"]
mod totp;
#[path = "support/wallet.rs"]
mod wallet;

use std::collections::HashMap;
use std::time::Duration;

use axum::Extension;
use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Response;
use axum::routing::get;
use http_body_util::BodyExt;
use leptoskit_auth::Account;
use leptoskit_auth::Auth;
use leptoskit_auth::AuthContext;
use leptoskit_auth::AuthError;
use leptoskit_auth::Event;
use leptoskit_auth::EventKind;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::MergeOffer;
use leptoskit_auth::SessionResponse;
use leptoskit_auth::SiwsInput;
use leptoskit_auth::SolanaOptions;
use leptoskit_auth::Wallet;
use leptoskit_auth::WalletInfo;
use leptoskit_auth::WalletSettings;
use leptoskit_db::Connection;
use leptoskit_db::Db;
use leptoskit_db::Dialect;
use leptoskit_db::Id;
use leptoskit_db::Pubkey;
use leptoskit_db::Statement;
use leptoskit_db::Value as DbValue;
use leptoskit_db::policy;
use leptoskit_db::table;
use postgres::PostgresCluster;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt;
use wallet::SoftwareWallet;

const ORIGIN: &str = "https://notes.example";

/// A forged answer to a challenge.
type Forgery<'a> = Box<dyn Fn(&SiwsInput) -> Value + 'a>;

/// An app table whose rows belong to whoever holds a wallet.
#[table(name = "vaults")]
#[policy(select, |vault, cx| vault.treasury.in_list(cx.wallets()))]
pub struct Vault {
	pub id: Id<Self>,
	pub treasury: Pubkey,
	pub name: String,
}

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

struct App {
	auth: Auth,
	router: Router,
	mailbox: Mailbox,
}

fn app(db: &Db, sign_in: bool) -> App {
	let mailbox = Mailbox::new();
	let auth = Auth::builder(db, Keys::single("test", [5; 32]))
		.app_name("Notes")
		.origin(ORIGIN)
		.client_ip_header("x-client-ip")
		.mailer(mailbox.clone())
		.solana(SolanaOptions {
			sign_in,
			chain_id: "devnet".into(),
		})
		.build()
		.unwrap();
	let vaults = Router::new().route(
		"/vaults",
		get(|Extension(db): Extension<Db>| {
			async move {
				let names: Vec<String> = Vault::query()
					.order_by(|vault| vault.name.asc())
					.all(&db)
					.await
					.unwrap()
					.into_iter()
					.map(|vault| vault.name)
					.collect();
				Json(names)
			}
		}),
	);
	let router = Router::new()
		.merge(vaults)
		.merge(auth.router())
		.layer(auth.layer());
	App {
		auth,
		router,
		mailbox,
	}
}

/// A browser: a cookie jar, and requests from the app's own pages.
#[derive(Default)]
struct Browser {
	jar: HashMap<String, String>,
	bearer: Option<String>,
}

impl Browser {
	async fn send(
		&mut self,
		app: &App,
		request: axum::http::request::Builder,
		body: Body,
	) -> Response {
		let cookies = self
			.jar
			.iter()
			.map(|(name, value)| format!("{name}={value}"))
			.collect::<Vec<_>>()
			.join("; ");
		let mut request = request
			.header("user-agent", "Firefox/131.0")
			.header("x-client-ip", "203.0.113.7");

		if !cookies.is_empty() {
			request = request.header(header::COOKIE, cookies);
		}

		if let Some(token) = &self.bearer {
			request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
		}

		let response = app
			.router
			.clone()
			.oneshot(request.body(body).unwrap())
			.await
			.unwrap();

		for value in response.headers().get_all(header::SET_COOKIE) {
			let value = value.to_str().unwrap();
			let (pair, attributes) = value.split_once(';').unwrap_or((value, ""));
			let (name, cookie) = pair.split_once('=').unwrap();

			if cookie.is_empty() || attributes.contains("Max-Age=0") {
				self.jar.remove(name);
			} else {
				self.jar.insert(name.to_owned(), cookie.to_owned());
			}
		}

		response
	}

	async fn post(&mut self, app: &App, path: &str, body: Value) -> Response {
		let mut request = Request::post(path).header(header::CONTENT_TYPE, "application/json");

		if self.bearer.is_none() {
			request = request.header(header::ORIGIN, ORIGIN);
		}

		self.send(app, request, Body::from(body.to_string())).await
	}

	async fn get(&mut self, app: &App, path: &str) -> Response {
		self.send(app, Request::get(path), Body::empty()).await
	}

	/// Links `wallet`, as the client does: options, sign, link.
	async fn link(&mut self, app: &App, wallet: &SoftwareWallet) -> Value {
		let input = self.link_input(app).await;
		let proof = wallet.sign_in(&input);
		json_body(
			self.post(
				app,
				"/auth/wallets/link",
				json!({ "proof": proof, "label": " Treasury " }),
			)
			.await,
		)
		.await
	}

	async fn link_input(&mut self, app: &App) -> SiwsInput {
		let options = self
			.post(app, "/auth/wallets/link/options", json!({}))
			.await;
		assert_eq!(options.status(), StatusCode::OK);
		serde_json::from_value(json_body(options).await["input"].clone()).unwrap()
	}

	async fn wallets(&mut self, app: &App) -> Vec<WalletInfo> {
		serde_json::from_value(json_body(self.get(app, "/auth/wallets").await).await).unwrap()
	}

	fn signed_in(&self) -> bool {
		self.jar.contains_key("__Host-session")
	}
}

async fn json_body(response: Response) -> Value {
	let bytes = response.into_body().collect().await.unwrap().to_bytes();
	serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn account_of(browser: &mut Browser, app: &App) -> Id<Account> {
	let session: SessionResponse =
		serde_json::from_value(json_body(browser.get(app, "/auth/session").await).await).unwrap();
	session.session.unwrap().account_id
}

async fn email_sign_in(app: &App, browser: &mut Browser, address: &str) {
	browser
		.post(app, "/auth/email/send", json!({ "email": address }))
		.await;
	let code = app
		.mailbox
		.latest_to(address)
		.unwrap()
		.subject
		.rsplit(' ')
		.next()
		.unwrap()
		.to_owned();
	let response = browser
		.post(
			app,
			"/auth/email/verify",
			json!({ "email": address, "code": code }),
		)
		.await;
	assert_eq!(response.status(), StatusCode::OK);
}

/// Runs `UPDATE auth.<table> SET <assignments> WHERE <key> = <value>`.
async fn age(db: &Db, table: &str, assignments: &str, key: &str, values: [DbValue; 2]) {
	let system = db.system("test: age a row");
	let (table, first, second) = match system.dialect() {
		Dialect::Postgres => (format!("auth.{table}"), "$1", "$2"),
		Dialect::Sqlite => (format!("auth_{table}"), "?1", "?2"),
	};
	let assignments = assignments.replace("$1", first);
	system
		.execute(Statement {
			sql: format!("UPDATE {table} SET {assignments} WHERE {key} = {second}"),
			args: values.to_vec(),
		})
		.await
		.unwrap();
}

fn minutes_ago(minutes: i64) -> DbValue {
	DbValue::Timestamp(leptoskit_db::now() - chrono::TimeDelta::minutes(minutes))
}

/// Makes the session's sign-in older than the step-up window.
async fn go_stale(db: &Db, account: Id<Account>) {
	age(
		db,
		"sessions",
		"authenticated_at = $1",
		"account_id",
		[minutes_ago(30), DbValue::Uuid(account.uuid())],
	)
	.await;
}

async fn events(db: &Db, account: Id<Account>, kind: EventKind) -> u64 {
	Event::query()
		.filter(|event| event.account_id.eq(Some(account)) & event.kind.eq(kind))
		.count(&db.system("test"))
		.await
		.unwrap()
}

#[tokio::test]
async fn a_wallet_links_with_a_fresh_proof_that_works_once() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(&db, false);
		let mut browser = Browser::default();
		let settings: WalletSettings = serde_json::from_value(
			json_body(browser.get(&app, "/auth/wallets/config").await).await,
		)
		.unwrap();
		assert_eq!(
			settings,
			WalletSettings {
				enabled: true,
				sign_in: false,
				chain_id: "devnet".into()
			}
		);

		// Linking needs someone signed in.
		let refused = browser
			.post(&app, "/auth/wallets/link/options", json!({}))
			.await;
		assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);

		email_sign_in(&app, &mut browser, "ada@example.com").await;
		let account = account_of(&mut browser, &app).await;
		let wallet = SoftwareWallet::new();

		// The message is bound to this site, names the app, and carries a
		// single-use nonce and a five-minute expiry.
		let input = browser.link_input(&app).await;
		assert_eq!(input.domain, "notes.example");
		assert_eq!(input.uri.as_deref(), Some(ORIGIN));
		assert_eq!(
			input.statement.as_deref(),
			Some("Link this wallet to your account on Notes.")
		);
		assert_eq!(input.chain_id.as_deref(), Some("devnet"));
		assert_eq!(input.request_id.as_deref(), Some("link"));
		assert!(input.nonce.as_ref().unwrap().len() >= 16);
		let issued =
			chrono::DateTime::parse_from_rfc3339(input.issued_at.as_deref().unwrap()).unwrap();
		let expires =
			chrono::DateTime::parse_from_rfc3339(input.expiration_time.as_deref().unwrap())
				.unwrap();
		assert_eq!(expires - issued, chrono::TimeDelta::minutes(5));

		let proof = wallet.sign_in(&input);
		let linked = browser
			.post(
				&app,
				"/auth/wallets/link",
				json!({ "proof": proof, "label": " Treasury " }),
			)
			.await;
		assert_eq!(linked.status(), StatusCode::OK);
		let linked = json_body(linked).await;
		assert_eq!(linked["outcome"], "linked");
		assert_eq!(linked["wallet"]["address"], wallet.address().to_string());
		assert_eq!(linked["wallet"]["label"], "Treasury");
		assert_eq!(linked["wallet"]["can_sign_in"], false);
		assert_eq!(linked["wallet"]["is_public"], false);

		// The proof is kept for audit; the owner is told.
		let row = Wallet::query()
			.filter(|row| row.account_id.eq(account))
			.first(&db.system("test"))
			.await
			.unwrap()
			.unwrap();
		assert_eq!(row.proof, proof.message);
		assert_eq!(row.proof_signature, proof.signature);
		assert_eq!(events(&db, account, EventKind::WalletLinked).await, 1);
		assert!(
			app.mailbox
				.latest_to("ada@example.com")
				.unwrap()
				.text
				.contains(&wallet.address().to_string())
		);

		// The same answer again: its nonce is spent.
		let replayed = browser
			.post(&app, "/auth/wallets/link", json!({ "proof": proof }))
			.await;
		assert_eq!(json_body(replayed).await["error"], "wallet_failed");

		// A wallet already linked here gets a fresh proof.
		let again = browser.link(&app, &wallet).await;
		assert_eq!(again["outcome"], "already_linked");
		assert_eq!(browser.wallets(&app).await.len(), 1);
		assert_eq!(events(&db, account, EventKind::WalletProved).await, 1);

		// Each answer that doesn't check out spends its challenge, too.
		let other = SoftwareWallet::new();
		let forged_cases: Vec<Forgery<'_>> = vec![
			// Signed for another site.
			Box::new(|input: &SiwsInput| {
				let elsewhere = SiwsInput {
					domain: "evil.example".into(),
					..input.clone()
				};
				json!({ "proof": other.sign_in(&elsewhere) })
			}),
			// Another nonce.
			Box::new(|input: &SiwsInput| {
				let mut changed = input.clone();
				changed.nonce = Some("SomeOtherNonce123".into());
				json!({ "proof": other.sign_in(&changed) })
			}),
			// Another key, claiming the address.
			Box::new(|input: &SiwsInput| {
				let mut proof = SoftwareWallet::new().sign_in(input);
				proof.address = other.address();
				proof.message = input.message(&other.address());
				json!({ "proof": proof })
			}),
			// Plain text, not a SIWS message.
			Box::new(|_: &SiwsInput| json!({ "proof": other.sign_text("let me in") })),
		];

		for forged in forged_cases {
			let input = browser.link_input(&app).await;
			let response = browser
				.post(&app, "/auth/wallets/link", forged(&input))
				.await;
			assert_eq!(response.status(), StatusCode::BAD_REQUEST);
			assert_eq!(json_body(response).await["error"], "wallet_failed");
			// The honest answer to that challenge no longer works either.
			let late = browser
				.post(
					&app,
					"/auth/wallets/link",
					json!({ "proof": other.sign_in(&input) }),
				)
				.await;
			assert_eq!(json_body(late).await["error"], "wallet_failed");
		}

		assert_eq!(events(&db, account, EventKind::WalletFailed).await, 4);
		assert_eq!(browser.wallets(&app).await.len(), 1);

		// An expired challenge is refused.
		let input = browser.link_input(&app).await;
		age(
			&db,
			"challenges",
			"expires_at = $1",
			"purpose",
			[minutes_ago(1), DbValue::Text("wallet_link".into())],
		)
		.await;
		let late = browser
			.post(
				&app,
				"/auth/wallets/link",
				json!({ "proof": other.sign_in(&input) }),
			)
			.await;
		assert_eq!(json_body(late).await["error"], "wallet_failed");

		// Linking is a change to how the account signs in: it needs a recent
		// sign-in.
		go_stale(&db, account).await;
		let stale = browser
			.post(&app, "/auth/wallets/link/options", json!({}))
			.await;
		assert_eq!(json_body(stale).await["error"], "reauthenticate");
	}
}

#[tokio::test]
async fn policies_name_the_owner_of_a_wallet() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(&db, false);
		let mut ada = Browser::default();
		email_sign_in(&app, &mut ada, "ada@example.com").await;
		let wallet = SoftwareWallet::new();
		assert_eq!(ada.link(&app, &wallet).await["outcome"], "linked");
		let stranger = SoftwareWallet::new();
		let system = db.system("test");

		for (treasury, name) in [
			(wallet.address(), "Ada's vault"),
			(stranger.address(), "Someone else's"),
		] {
			NewVault {
				treasury,
				name: name.into(),
			}
			.insert(&system)
			.await
			.unwrap();
		}

		// Through the layer: each request's policies see its wallets.
		let names: Vec<String> =
			serde_json::from_value(json_body(ada.get(&app, "/vaults").await).await).unwrap();
		assert_eq!(names, ["Ada's vault"]);
		let mut nobody = Browser::default();
		let names: Vec<String> =
			serde_json::from_value(json_body(nobody.get(&app, "/vaults").await).await).unwrap();
		assert_eq!(names, [] as [String; 0]);
		let mut grace = Browser::default();
		email_sign_in(&app, &mut grace, "grace@example.com").await;
		let names: Vec<String> =
			serde_json::from_value(json_body(grace.get(&app, "/vaults").await).await).unwrap();
		assert_eq!(names, [] as [String; 0]);

		// The auth layer adds wallets only beside a signed-in person.
		let account = account_of(&mut ada, &app).await;
		let me = leptoskit_auth::SignedIn {
			account,
			session: Id::new(),
			aal: leptoskit_auth::Aal::One,
			authenticated_at: leptoskit_db::now(),
			mfa_at: None,
		};
		let context = app.auth.request_context(Some(me)).await.unwrap();
		assert_eq!(
			context
				.get::<leptoskit_auth::Wallets>()
				.map(leptoskit_auth::Wallets::addresses),
			Some(&[wallet.address()][..])
		);
		let visible = Vault::query()
			.count(&db.with_context(context))
			.await
			.unwrap();
		assert_eq!(visible, 1);
	}
}

#[tokio::test]
async fn wallets_sign_in_only_when_their_owner_and_the_server_allow_it() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		// Off on the server: the routes say so, and the setting can't be
		// turned on.
		let off = app(&db, false);
		let mut ada = Browser::default();
		let refused = ada
			.post(&off, "/auth/wallets/sign-in/options", json!({}))
			.await;
		assert_eq!(json_body(refused).await["error"], "unsupported");
		email_sign_in(&off, &mut ada, "ada@example.com").await;
		let wallet = SoftwareWallet::new();
		let linked = ada.link(&off, &wallet).await;
		let id = linked["wallet"]["id"].clone();
		let refused = ada
			.post(
				&off,
				"/auth/wallets/update",
				json!({ "id": id, "can_sign_in": true }),
			)
			.await;
		assert_eq!(json_body(refused).await["error"], "unsupported");
		let finish = ada
			.post(
				&off,
				"/auth/wallets/sign-in",
				json!({ "proof": wallet.sign_text("hello") }),
			)
			.await;
		assert_eq!(json_body(finish).await["error"], "wallet_failed");
		let token = leptoskit_auth::SessionToken::new("unknown");
		let finish = off
			.auth
			.finish_wallet_sign_in(
				&token,
				&wallet.sign_text("hello"),
				&leptoskit_auth::RequestInfo::default(),
			)
			.await;
		assert!(
			matches!(finish, Err(AuthError::Unsupported(_))),
			"{finish:?}"
		);

		// On: a linked wallet still can't sign in until its owner says so.
		let app = app(&db, true);
		let mut stranger = Browser::default();
		let started = stranger
			.post(&app, "/auth/wallets/sign-in/options", json!({}))
			.await;
		assert!(stranger.jar.contains_key("__Host-wallet"));
		let input: SiwsInput =
			serde_json::from_value(json_body(started).await["input"].clone()).unwrap();
		assert_eq!(input.statement.as_deref(), Some("Sign in to Notes."));

		// A request that says nothing of where it's from, but carries the
		// challenge's cookie, is a browser's, from another site.
		let cross_site = stranger
			.send(
				&app,
				Request::post("/auth/wallets/sign-in")
					.header(header::CONTENT_TYPE, "application/json"),
				Body::from(json!({ "proof": wallet.sign_in(&input) }).to_string()),
			)
			.await;
		assert_eq!(cross_site.status(), StatusCode::FORBIDDEN);
		assert!(
			stranger.jar.contains_key("__Host-wallet"),
			"nothing was spent"
		);
		let response = stranger
			.post(
				&app,
				"/auth/wallets/sign-in",
				json!({ "proof": wallet.sign_in(&input) }),
			)
			.await;
		assert_eq!(json_body(response).await["error"], "unknown_wallet");
		assert!(
			!stranger.jar.contains_key("__Host-wallet"),
			"the challenge's cookie goes"
		);

		// Turning it on needs a recent sign-in, and emails the owner.
		let account = account_of(&mut ada, &app).await;
		go_stale(&db, account).await;
		let stale = ada
			.post(
				&app,
				"/auth/wallets/update",
				json!({ "id": id, "can_sign_in": true }),
			)
			.await;
		assert_eq!(json_body(stale).await["error"], "reauthenticate");
		// A label and visibility don't.
		let renamed = ada
			.post(
				&app,
				"/auth/wallets/update",
				json!({ "id": id, "label": "Cold storage", "is_public": true }),
			)
			.await;
		assert_eq!(json_body(renamed).await["label"], "Cold storage");
		let public = app.auth.public_wallets(account).await.unwrap();
		assert_eq!(public.len(), 1);
		assert_eq!(public[0].address, wallet.address());

		let mut ada = Browser::default();
		email_sign_in(&app, &mut ada, "ada@example.com").await;
		let on = ada
			.post(
				&app,
				"/auth/wallets/update",
				json!({ "id": id, "can_sign_in": true }),
			)
			.await;
		assert_eq!(json_body(on).await["can_sign_in"], true);
		assert_eq!(
			app.mailbox.latest_to("ada@example.com").unwrap().subject,
			"Notes: A wallet can sign in"
		);
		assert_eq!(
			events(&db, account, EventKind::WalletSignInChanged).await,
			1
		);

		// Turning it off again, with the email still a way in, tells the
		// owner too.
		let off = ada
			.post(
				&app,
				"/auth/wallets/update",
				json!({ "id": id, "can_sign_in": false }),
			)
			.await;
		assert_eq!(json_body(off).await["can_sign_in"], false);
		assert_eq!(
			app.mailbox.latest_to("ada@example.com").unwrap().subject,
			"Notes: A wallet can no longer sign in"
		);
	}
}

#[tokio::test]
async fn wallets_need_an_origin_to_bind_messages_to() {
	let db = leptoskit_db::testing::sqlite().await;
	let mailbox = Mailbox::new();
	let auth = Auth::builder(&db, Keys::single("test", [5; 32]))
		.mailer(mailbox)
		.solana(SolanaOptions {
			sign_in: true,
			..SolanaOptions::default()
		})
		.build()
		.unwrap();
	assert!(!auth.wallet_settings().enabled);
	let me = leptoskit_auth::SignedIn {
		account: Id::new(),
		session: Id::new(),
		aal: leptoskit_auth::Aal::One,
		authenticated_at: leptoskit_db::now(),
		mfa_at: None,
	};
	let request = leptoskit_auth::RequestInfo::default();
	let wallet = SoftwareWallet::new();

	for refused in [
		auth.begin_wallet_link(&me, &request).await.map(drop),
		auth.begin_wallet_proof(&me, &wallet.address(), &request)
			.await
			.map(drop),
		auth.begin_wallet_sign_in(&request).await.map(drop),
	] {
		assert!(
			matches!(
				refused,
				Err(AuthError::Unsupported(_) | AuthError::Config(_))
			),
			"{refused:?}"
		);
	}
}

/// Ada, signed in, with a linked wallet that may sign in.
async fn ada_with_a_signing_wallet(app: &App) -> (Browser, Id<Account>, SoftwareWallet) {
	let mut ada = Browser::default();
	email_sign_in(app, &mut ada, "ada@example.com").await;
	let account = account_of(&mut ada, app).await;
	let wallet = SoftwareWallet::new();
	let linked = ada.link(app, &wallet).await;
	let on = ada
		.post(
			app,
			"/auth/wallets/update",
			json!({ "id": linked["wallet"]["id"], "can_sign_in": true }),
		)
		.await;
	assert_eq!(on.status(), StatusCode::OK);
	(ada, account, wallet)
}

#[tokio::test]
async fn a_wallet_signs_in_as_one_factor() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(&db, true);
		let (mut ada, account, wallet) = ada_with_a_signing_wallet(&app).await;

		// Now it signs in, as one factor.
		let mut phone = Browser::default();
		let started = phone
			.post(&app, "/auth/wallets/sign-in/options", json!({}))
			.await;
		let input: SiwsInput =
			serde_json::from_value(json_body(started).await["input"].clone()).unwrap();
		let proof = wallet.sign_in(&input);

		// Without the browser's cookie the answer goes nowhere: a page on
		// another site can't finish a sign-in it didn't start here.
		let mut elsewhere = Browser::default();
		let response = elsewhere
			.post(&app, "/auth/wallets/sign-in", json!({ "proof": proof }))
			.await;
		assert_eq!(json_body(response).await["error"], "wallet_failed");

		let response = phone
			.post(&app, "/auth/wallets/sign-in", json!({ "proof": proof }))
			.await;
		assert_eq!(response.status(), StatusCode::OK);
		let body = json_body(response).await;
		assert_eq!(body["session"]["aal"], "aal1");
		assert!(phone.signed_in());
		assert_eq!(account_of(&mut phone, &app).await, account);

		// A native app gets its challenge's token and its session tokens in
		// the body.
		let mut native = Browser::default();
		let started = json_body(
			native
				.send(
					&app,
					Request::post("/auth/wallets/sign-in/options")
						.header(header::CONTENT_TYPE, "application/json"),
					Body::from(json!({ "transport": "bearer" }).to_string()),
				)
				.await,
		)
		.await;
		assert!(native.jar.is_empty());
		let input: SiwsInput = serde_json::from_value(started["input"].clone()).unwrap();
		let response = native
			.send(
				&app,
				Request::post("/auth/wallets/sign-in")
					.header(header::CONTENT_TYPE, "application/json"),
				Body::from(
					json!({
						"proof": wallet.sign_in(&input),
						"token": started["token"],
						"transport": "bearer",
						"device_name": "Ada's phone",
					})
					.to_string(),
				),
			)
			.await;
		let body = json_body(response).await;
		assert!(body["tokens"]["access_token"].is_string(), "{body}");
		assert_eq!(body["session"]["device"], "Ada's phone");

		// With two-step verification on, a wallet is only the first step.
		let setup = json_body(ada.post(&app, "/auth/totp", json!({})).await).await;
		let secret = setup["secret"].as_str().unwrap().to_owned();
		ada.post(
			&app,
			"/auth/totp/confirm",
			json!({ "code": totp::totp(&secret, 0) }),
		)
		.await;
		let mut laptop = Browser::default();
		let started = laptop
			.post(&app, "/auth/wallets/sign-in/options", json!({}))
			.await;
		let input: SiwsInput =
			serde_json::from_value(json_body(started).await["input"].clone()).unwrap();
		let response = laptop
			.post(
				&app,
				"/auth/wallets/sign-in",
				json!({ "proof": wallet.sign_in(&input) }),
			)
			.await;
		let body = json_body(response).await;
		assert_eq!(
			body["second_factor"]["factors"],
			json!(["totp", "recovery_code"])
		);
		assert!(laptop.jar.contains_key("__Host-second-factor"));
		assert!(!laptop.signed_in());

		// A disabled account can't sign in with its wallet.
		app.auth
			.disable(account, &leptoskit_auth::RequestInfo::default())
			.await
			.unwrap();
		let mut late = Browser::default();
		let started = late
			.post(&app, "/auth/wallets/sign-in/options", json!({}))
			.await;
		let input: SiwsInput =
			serde_json::from_value(json_body(started).await["input"].clone()).unwrap();
		let response = late
			.post(
				&app,
				"/auth/wallets/sign-in",
				json!({ "proof": wallet.sign_in(&input) }),
			)
			.await;
		assert_eq!(json_body(response).await["error"], "disabled");
	}
}

#[tokio::test]
async fn fresh_proofs_unlinking_and_the_last_way_in() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(&db, true);
		let mut ada = Browser::default();
		email_sign_in(&app, &mut ada, "ada@example.com").await;
		let account = account_of(&mut ada, &app).await;
		let wallet = SoftwareWallet::new();
		let linked = ada.link(&app, &wallet).await;
		let id = linked["wallet"]["id"].clone();
		let me = leptoskit_auth::SignedIn {
			account,
			session: Id::new(),
			aal: leptoskit_auth::Aal::One,
			authenticated_at: leptoskit_db::now(),
			mfa_at: None,
		};

		// A payout asks for a proof from the last two minutes.
		let window = Duration::from_secs(120);
		app.auth
			.require_wallet_proof(&me, &wallet.address(), window)
			.await
			.unwrap();
		age(
			&db,
			"wallets",
			"last_proved_at = $1",
			"account_id",
			[minutes_ago(3), DbValue::Uuid(account.uuid())],
		)
		.await;
		let stale = app
			.auth
			.require_wallet_proof(&me, &wallet.address(), window)
			.await;
		assert!(matches!(stale, Err(AuthError::ProofRequired)), "{stale:?}");
		let strangers = app
			.auth
			.require_wallet_proof(&me, &SoftwareWallet::new().address(), window)
			.await;
		assert!(matches!(strangers, Err(AuthError::NotFound)));

		// The proof names the wallet, and only it may answer.
		let options = ada
			.post(
				&app,
				"/auth/wallets/proof/options",
				json!({ "address": wallet.address() }),
			)
			.await;
		let input: SiwsInput =
			serde_json::from_value(json_body(options).await["input"].clone()).unwrap();
		assert_eq!(
			input.address.as_deref(),
			Some(wallet.address().as_inner().as_str())
		);
		assert_eq!(input.request_id.as_deref(), Some("proof"));
		let impostor = SoftwareWallet::new();
		let wrong = ada
			.post(&app, "/auth/wallets/proof", json!(impostor.sign_in(&input)))
			.await;
		assert_eq!(json_body(wrong).await["error"], "wallet_failed");
		let options = ada
			.post(
				&app,
				"/auth/wallets/proof/options",
				json!({ "address": wallet.address() }),
			)
			.await;
		let input: SiwsInput =
			serde_json::from_value(json_body(options).await["input"].clone()).unwrap();
		let proved = ada
			.post(&app, "/auth/wallets/proof", json!(wallet.sign_in(&input)))
			.await;
		assert_eq!(proved.status(), StatusCode::OK);
		app.auth
			.require_wallet_proof(&me, &wallet.address(), window)
			.await
			.unwrap();
		// Someone else's wallet isn't theirs to prove.
		let theirs = ada
			.post(
				&app,
				"/auth/wallets/proof/options",
				json!({ "address": impostor.address() }),
			)
			.await;
		assert_eq!(json_body(theirs).await["error"], "not_found");

		// With signing in on and the email gone, the wallet is the last way
		// in: it can't be turned off or unlinked.
		ada.post(
			&app,
			"/auth/wallets/update",
			json!({ "id": id, "can_sign_in": true }),
		)
		.await;
		let system = db.system("test");
		leptoskit_auth::EmailAddress::query()
			.filter(|email| email.account_id.eq(account))
			.delete(&system)
			.await
			.unwrap();
		let off = ada
			.post(
				&app,
				"/auth/wallets/update",
				json!({ "id": id, "can_sign_in": false }),
			)
			.await;
		assert_eq!(json_body(off).await["error"], "last_sign_in_method");
		let unlinked = ada
			.post(&app, "/auth/wallets/unlink", json!({ "id": id }))
			.await;
		assert_eq!(json_body(unlinked).await["error"], "last_sign_in_method");
		assert_eq!(ada.wallets(&app).await.len(), 1);

		// With another way in, it unlinks, after a recent sign-in.
		let second = SoftwareWallet::new();
		let other = ada.link(&app, &second).await;
		ada.post(
			&app,
			"/auth/wallets/update",
			json!({ "id": other["wallet"]["id"], "can_sign_in": true }),
		)
		.await;
		go_stale(&db, account).await;
		let stale = ada
			.post(&app, "/auth/wallets/unlink", json!({ "id": id }))
			.await;
		assert_eq!(json_body(stale).await["error"], "reauthenticate");
		age(
			&db,
			"sessions",
			"authenticated_at = $1",
			"account_id",
			[minutes_ago(0), DbValue::Uuid(account.uuid())],
		)
		.await;
		let unlinked = ada
			.post(&app, "/auth/wallets/unlink", json!({ "id": id }))
			.await;
		assert_eq!(unlinked.status(), StatusCode::NO_CONTENT);
		assert_eq!(ada.wallets(&app).await.len(), 1);
		assert_eq!(events(&db, account, EventKind::WalletUnlinked).await, 1);
		let missing = ada
			.post(&app, "/auth/wallets/unlink", json!({ "id": id }))
			.await;
		assert_eq!(json_body(missing).await["error"], "not_found");
	}
}

#[tokio::test]
async fn a_wallet_on_another_account_merges_only_when_it_signs_in_there() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(&db, true);
		let wallet = SoftwareWallet::new();
		let mut grace = Browser::default();
		email_sign_in(&app, &mut grace, "grace@example.com").await;
		let absorbed = account_of(&mut grace, &app).await;
		let linked = grace.link(&app, &wallet).await;
		let id = linked["wallet"]["id"].clone();

		// It can't sign in to Grace's account, so signing with it proves
		// nothing about that account.
		let mut ada = Browser::default();
		email_sign_in(&app, &mut ada, "ada@example.com").await;
		let survivor = account_of(&mut ada, &app).await;
		let input = ada.link_input(&app).await;
		let response = ada
			.post(
				&app,
				"/auth/wallets/link",
				json!({ "proof": wallet.sign_in(&input) }),
			)
			.await;
		assert_eq!(response.status(), StatusCode::CONFLICT);
		assert_eq!(json_body(response).await["error"], "wallet_in_use");

		// Once it may sign in there, it's as good as signing in: a merge
		// offer.
		grace
			.post(
				&app,
				"/auth/wallets/update",
				json!({ "id": id, "can_sign_in": true }),
			)
			.await;
		assert_eq!(ada.link(&app, &wallet).await["outcome"], "merge");
		let offer: Option<MergeOffer> =
			serde_json::from_value(json_body(ada.get(&app, "/auth/merge").await).await).unwrap();
		let offer = offer.unwrap();
		assert_eq!(offer.provider, "solana");
		assert_eq!(offer.wallets, [wallet.address()]);
		assert_eq!(offer.emails, ["grace@example.com"]);

		let merged = ada
			.post(
				&app,
				"/auth/merge",
				json!({ "mode": "move_login", "confirm": true }),
			)
			.await;
		assert_eq!(merged.status(), StatusCode::NO_CONTENT);
		let wallets = ada.wallets(&app).await;
		assert_eq!(wallets.len(), 1);
		assert!(wallets[0].can_sign_in);
		assert_eq!(app.auth.resolve_merged(absorbed).await.unwrap(), survivor);
		assert!(
			Account::find(&db.system("test"), absorbed)
				.await
				.unwrap()
				.is_none()
		);
	}
}

#[test]
fn wallet_rules_export_as_row_level_security() {
	let schema = leptoskit_db::schema_with_row_security(Dialect::Postgres).unwrap();
	let vaults = schema
		.tables
		.iter()
		.find(|table| table.name.name == "vaults")
		.and_then(|table| table.row_security.as_ref())
		.unwrap();
	assert_eq!(
		vaults.policies[0].using.as_deref(),
		Some("\"treasury\" = ANY(NULLIF(current_setting('leptoskit.wallets', true), '')::text[])")
	);
}
