//! Merging accounts in "bring its data" mode, against an app's own tables:
//! each table's `on_account_merge` strategy, the preview's counts matching
//! what moves, the app's own `OnMerge`, a failure halfway through undoing
//! everything, and the proofs a merge still needs.

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
use std::sync::Mutex;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Response;
use http_body_util::BodyExt;
use leptoskit_auth::Account;
use leptoskit_auth::Auth;
use leptoskit_auth::AuthError;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::MergeData;
use leptoskit_auth::MergeFate;
use leptoskit_auth::MergeMode;
use leptoskit_auth::MergeOffer;
use leptoskit_auth::OnMerge;
use leptoskit_auth::SessionResponse;
use leptoskit_auth::SiwsInput;
use leptoskit_auth::SolanaOptions;
use leptoskit_auth::Wallet;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::Tx;
use leptoskit_db::merge::TableName;
use leptoskit_db::table;
use postgres::PostgresCluster;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt;
use wallet::SoftwareWallet;

const ORIGIN: &str = "https://notes.example";

/// The app's user, keyed by the account: the survivor's row stays.
#[table(name = "users")]
pub struct User {
	#[db(on_delete = cascade)]
	pub id: Id<Account>,
	pub name: String,
	pub bio: Option<String>,
}

/// Plain rows: they move.
#[table(name = "posts")]
pub struct Post {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub author_id: Id<Account>,
	pub title: String,
}

/// One per pair: the survivor's duplicates win.
#[table(
	name = "follows",
	unique(follower_id, topic),
	on_account_merge = reassign_skip_conflicts
)]
pub struct Follow {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub follower_id: Id<Account>,
	pub topic: String,
}

/// One per account: the newer one stays.
#[table(name = "prefs", on_account_merge = keep_newest)]
pub struct Pref {
	pub id: Id<Self>,
	#[db(unique, on_delete = cascade)]
	pub user_id: Id<Account>,
	pub theme: String,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// Dropped: the survivor's own count.
#[table(name = "drafts", on_account_merge = keep_survivor)]
pub struct Draft {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub user_id: Id<Account>,
}

/// One per account, added together by the app.
#[table(name = "balances", on_account_merge = custom)]
pub struct Balance {
	pub id: Id<Self>,
	#[db(unique, on_delete = cascade)]
	pub user_id: Id<Account>,
	pub cents: i64,
}

/// Two references to the account in one table: counted together.
#[table(name = "gifts")]
pub struct Gift {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub from_id: Id<Account>,
	#[db(on_delete = cascade)]
	pub to_id: Id<Account>,
}

/// One of each kind per account, moved plainly: a duplicate stops the
/// merge.
#[table(name = "badges", unique(user_id, kind), on_account_merge = reassign)]
pub struct Badge {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub user_id: Id<Account>,
	pub kind: String,
}

/// An app that words the preview itself, and copies nothing from the other
/// account's profile.
pub struct Worded;

impl OnMerge for Worded {
	async fn summary(
		db: &Db,
		absorbed: Id<Account>,
	) -> leptoskit_auth::Result<Option<Vec<MergeData>>> {
		let posts = Post::query()
			.filter(|post| post.author_id.eq(absorbed))
			.count(db)
			.await?;
		Ok(Some(vec![MergeData {
			label: "stories".into(),
			count: posts,
			fate: MergeFate::Moves,
		}]))
	}

	async fn merge_custom(
		tx: &Tx,
		_table: &TableName,
		_survivor: Id<Account>,
		absorbed: Id<Account>,
	) -> leptoskit_auth::Result<()> {
		Balance::query()
			.filter(|balance| balance.user_id.eq(absorbed))
			.delete(tx)
			.await?;
		Ok(())
	}
}

/// Survivors whose merges the app's code fails, for the test that a
/// failure halfway undoes everything.
static FAIL_FOR: Mutex<Vec<Id<Account>>> = Mutex::new(Vec::new());

impl OnMerge for User {
	async fn merge_profile(
		tx: &Tx,
		survivor: Id<Account>,
		absorbed: Id<Account>,
	) -> leptoskit_auth::Result<()> {
		let old = User::get(tx, absorbed).await?;
		let mine = User::get(tx, survivor).await?;

		if mine.bio.is_none() {
			User::update()
				.filter(|user| user.id.eq(survivor))
				.set(|user| user.bio, old.bio)
				.run(tx)
				.await?;
		}

		Ok(())
	}

	async fn merge_custom(
		tx: &Tx,
		table: &TableName,
		survivor: Id<Account>,
		absorbed: Id<Account>,
	) -> leptoskit_auth::Result<()> {
		assert_eq!(table.name, "balances");

		if FAIL_FOR.lock().unwrap().contains(&survivor) {
			return Err(AuthError::sign_up("the ledger is closed"));
		}

		let Some(theirs) = Balance::query()
			.filter(|balance| balance.user_id.eq(absorbed))
			.first(tx)
			.await?
		else {
			return Ok(());
		};
		Balance::query()
			.filter(|balance| balance.id.eq(theirs.id))
			.delete(tx)
			.await?;
		let mine = Balance::query()
			.filter(|balance| balance.user_id.eq(survivor))
			.first(tx)
			.await?;

		match mine {
			Some(mine) => {
				Balance::update()
					.filter(|balance| balance.id.eq(mine.id))
					.set(|balance| balance.cents, mine.cents + theirs.cents)
					.run(tx)
					.await?;
			}
			None => {
				NewBalance {
					user_id: survivor,
					cents: theirs.cents,
				}
				.insert(tx)
				.await?;
			}
		}

		Ok(())
	}
}

impl leptoskit_auth::OnSignUp for User {
	async fn on_sign_up(
		tx: &Tx,
		account: Id<Account>,
		profile: &leptoskit_auth::Profile,
	) -> leptoskit_auth::Result<()> {
		NewUser {
			id: account,
			name: profile
				.email
				.as_ref()
				.map(ToString::to_string)
				.unwrap_or_default(),
			bio: None,
		}
		.insert(tx)
		.await?;
		Ok(())
	}
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

fn app(db: &Db, hooks: bool) -> App {
	app_with(db, |builder| {
		if hooks {
			builder.on_merge::<User>()
		} else {
			builder
		}
	})
}

fn app_with(
	db: &Db,
	hooks: impl FnOnce(leptoskit_auth::AuthBuilder) -> leptoskit_auth::AuthBuilder,
) -> App {
	let mailbox = Mailbox::new();
	let builder = Auth::builder(db, Keys::single("test", [7; 32]))
		.app_name("Notes")
		.origin(ORIGIN)
		.mailer(mailbox.clone())
		.on_sign_up::<User>()
		.solana(SolanaOptions {
			sign_in: true,
			..SolanaOptions::default()
		});

	let auth = hooks(builder).build().unwrap();
	let router = Router::new().merge(auth.router()).layer(auth.layer());
	App {
		auth,
		router,
		mailbox,
	}
}

#[derive(Default)]
struct Browser {
	jar: HashMap<String, String>,
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
		let mut request = request.header("user-agent", "Firefox/131.0");

		if !cookies.is_empty() {
			request = request.header(header::COOKIE, cookies);
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
		let request = Request::post(path)
			.header(header::ORIGIN, ORIGIN)
			.header(header::CONTENT_TYPE, "application/json");
		self.send(app, request, Body::from(body.to_string())).await
	}

	async fn get(&mut self, app: &App, path: &str) -> Response {
		self.send(app, Request::get(path), Body::empty()).await
	}

	async fn link(&mut self, app: &App, wallet: &SoftwareWallet) -> Value {
		let options = self
			.post(app, "/auth/wallets/link/options", json!({}))
			.await;
		let input: SiwsInput =
			serde_json::from_value(json_body(options).await["input"].clone()).unwrap();
		json_body(
			self.post(
				app,
				"/auth/wallets/link",
				json!({ "proof": wallet.sign_in(&input) }),
			)
			.await,
		)
		.await
	}

	async fn offer(&mut self, app: &App) -> Option<MergeOffer> {
		serde_json::from_value(json_body(self.get(app, "/auth/merge").await).await).unwrap()
	}
}

/// Moves the account's sign-ins `minutes` into the past.
async fn signed_in_at(db: &Db, account: Id<Account>, minutes: i64) {
	use leptoskit_db::Connection;

	let system = db.system("test: age a session");

	let (table, first, second) = match system.dialect() {
		leptoskit_db::Dialect::Postgres => ("auth.sessions", "$1", "$2"),
		leptoskit_db::Dialect::Sqlite => ("auth_sessions", "?1", "?2"),
	};

	system
		.execute(leptoskit_db::Statement {
			sql: format!(
				"UPDATE {table} SET authenticated_at = {first} WHERE account_id = {second}"
			),
			args: vec![
				leptoskit_db::Value::Timestamp(
					leptoskit_db::now() - chrono::TimeDelta::minutes(minutes),
				),
				leptoskit_db::Value::Uuid(account.uuid()),
			],
		})
		.await
		.unwrap();
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

/// Two people, each with data, the second's wallet able to sign in, and
/// the first holding a merge offer for the second's account.
struct Pair {
	survivor: Browser,
	survivor_id: Id<Account>,
	absorbed: Browser,
	absorbed_id: Id<Account>,
	wallet: SoftwareWallet,
}

async fn pair(app: &App, db: &Db) -> Pair {
	let system = db.system("test");
	let mut absorbed = Browser::default();
	email_sign_in(app, &mut absorbed, "old@example.com").await;
	let absorbed_id = account_of(&mut absorbed, app).await;
	let mut survivor = Browser::default();
	email_sign_in(app, &mut survivor, "ada@example.com").await;
	let survivor_id = account_of(&mut survivor, app).await;

	User::update()
		.filter(|user| user.id.eq(absorbed_id))
		.set(|user| user.bio, Some("Counts things.".to_owned()))
		.run(&system)
		.await
		.unwrap();

	for (author, title) in [
		(survivor_id, "Mine"),
		(absorbed_id, "Old one"),
		(absorbed_id, "Old two"),
	] {
		NewPost {
			author_id: author,
			title: title.into(),
		}
		.insert(&system)
		.await
		.unwrap();
	}

	for (follower, topic) in [
		(survivor_id, "rust"),
		(absorbed_id, "rust"),
		(absorbed_id, "solana"),
	] {
		NewFollow {
			follower_id: follower,
			topic: topic.into(),
		}
		.insert(&system)
		.await
		.unwrap();
	}

	NewPref {
		user_id: survivor_id,
		theme: "light".into(),
	}
	.insert(&system)
	.await
	.unwrap();
	tokio::time::sleep(std::time::Duration::from_millis(5)).await;
	NewPref {
		user_id: absorbed_id,
		theme: "dark".into(),
	}
	.insert(&system)
	.await
	.unwrap();
	NewDraft {
		user_id: absorbed_id,
	}
	.insert(&system)
	.await
	.unwrap();

	for (user, cents) in [(survivor_id, 500), (absorbed_id, 250)] {
		NewBalance {
			user_id: user,
			cents,
		}
		.insert(&system)
		.await
		.unwrap();
	}

	let wallet = SoftwareWallet::new();
	let linked = absorbed.link(app, &wallet).await;
	absorbed
		.post(
			app,
			"/auth/wallets/update",
			json!({ "id": linked["wallet"]["id"], "can_sign_in": true }),
		)
		.await;
	assert_eq!(survivor.link(app, &wallet).await["outcome"], "merge");

	Pair {
		survivor,
		survivor_id,
		absorbed,
		absorbed_id,
		wallet,
	}
}

async fn snapshot(db: &Db, account: Id<Account>) -> (u64, Vec<String>, String, u64, i64) {
	let system = db.system("test");
	let posts = Post::query()
		.filter(|post| post.author_id.eq(account))
		.count(&system)
		.await
		.unwrap();
	let mut topics: Vec<String> = Follow::query()
		.filter(|follow| follow.follower_id.eq(account))
		.all(&system)
		.await
		.unwrap()
		.into_iter()
		.map(|follow| follow.topic)
		.collect();
	topics.sort();
	let theme = Pref::query()
		.filter(|pref| pref.user_id.eq(account))
		.first(&system)
		.await
		.unwrap()
		.map(|pref| pref.theme)
		.unwrap_or_default();
	let drafts = Draft::query()
		.filter(|draft| draft.user_id.eq(account))
		.count(&system)
		.await
		.unwrap();
	let cents = Balance::query()
		.filter(|balance| balance.user_id.eq(account))
		.first(&system)
		.await
		.unwrap()
		.map_or(0, |balance| balance.cents);
	(posts, topics, theme, drafts, cents)
}

#[tokio::test]
async fn bringing_the_data_moves_each_table_its_own_way() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(&db, true);
		let mut pair = pair(&app, &db).await;
		NewGift {
			from_id: pair.absorbed_id,
			to_id: pair.absorbed_id,
		}
		.insert(&db.system("test"))
		.await
		.unwrap();

		// The preview says what will happen, table by table; a table that
		// points at the account twice counts both.
		let offer = pair.survivor.offer(&app).await.unwrap();
		assert_eq!(offer.modes, [MergeMode::BringData, MergeMode::MoveLogin]);
		assert_eq!(offer.wallets, [pair.wallet.address()]);
		let data = |label: &str, count: u64, fate: MergeFate| {
			MergeData {
				label: label.into(),
				count,
				fate,
			}
		};
		let mut preview = offer.data.clone();
		preview.sort_by(|left, right| left.label.cmp(&right.label));
		assert_eq!(
			preview,
			[
				data("balances", 1, MergeFate::AppDecides),
				data("drafts", 1, MergeFate::Dropped),
				data("follows", 2, MergeFate::MovesUnlessDuplicate),
				data("gifts", 2, MergeFate::Moves),
				data("posts", 2, MergeFate::Moves),
				data("prefs", 1, MergeFate::NewestStays),
			]
		);

		// A stale sign-in can't merge.
		let system = db.system("test");
		signed_in_at(&db, pair.survivor_id, 30).await;
		let stale = pair
			.survivor
			.post(
				&app,
				"/auth/merge",
				json!({ "mode": "bring_data", "confirm": true }),
			)
			.await;
		assert_eq!(json_body(stale).await["error"], "reauthenticate");
		signed_in_at(&db, pair.survivor_id, 0).await;

		let merged = pair
			.survivor
			.post(
				&app,
				"/auth/merge",
				json!({ "mode": "bring_data", "confirm": true }),
			)
			.await;
		assert_eq!(
			merged.status(),
			StatusCode::NO_CONTENT,
			"{}",
			json_body(merged).await
		);

		// What moved matches the preview.
		assert_eq!(
			snapshot(&db, pair.survivor_id).await,
			(
				3,
				vec!["rust".to_owned(), "solana".to_owned()],
				"dark".to_owned(),
				0,
				750
			)
		);
		assert_eq!(
			snapshot(&db, pair.absorbed_id).await,
			(0, Vec::new(), String::new(), 0, 0)
		);
		let me = User::get(&system, pair.survivor_id).await.unwrap();
		assert_eq!(me.bio.as_deref(), Some("Counts things."));
		assert!(
			User::find(&system, pair.absorbed_id)
				.await
				.unwrap()
				.is_none()
		);

		// Sign-in methods moved too; the other account is a tombstone, and
		// its sessions are gone.
		let wallet = Wallet::query()
			.filter(|wallet| wallet.account_id.eq(pair.survivor_id))
			.count(&system)
			.await
			.unwrap();
		assert_eq!(wallet, 1);
		assert_eq!(
			app.auth.resolve_merged(pair.absorbed_id).await.unwrap(),
			pair.survivor_id
		);
		let session = pair.absorbed.get(&app, "/auth/session").await;
		assert_eq!(json_body(session).await["session"], Value::Null);
		assert!(
			app.mailbox
				.latest_to("ada@example.com")
				.unwrap()
				.text
				.contains("sign-in methods and data were moved")
		);
	}
}

#[tokio::test]
async fn a_failure_halfway_through_changes_nothing() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(&db, true);
		let mut pair = pair(&app, &db).await;
		let before = (
			snapshot(&db, pair.survivor_id).await,
			snapshot(&db, pair.absorbed_id).await,
		);

		// The app's code fails after the posts, follows, and prefs moved,
		// in the same transaction.
		FAIL_FOR.lock().unwrap().push(pair.survivor_id);
		let failed = pair
			.survivor
			.post(
				&app,
				"/auth/merge",
				json!({ "mode": "bring_data", "confirm": true }),
			)
			.await;
		assert_eq!(failed.status(), StatusCode::UNPROCESSABLE_ENTITY);

		let after = (
			snapshot(&db, pair.survivor_id).await,
			snapshot(&db, pair.absorbed_id).await,
		);
		assert_eq!(before, after);
		let system = db.system("test");
		assert!(
			Account::find(&system, pair.absorbed_id)
				.await
				.unwrap()
				.is_some()
		);
		let wallets = Wallet::query()
			.filter(|wallet| wallet.account_id.eq(pair.absorbed_id))
			.count(&system)
			.await
			.unwrap();
		assert_eq!(wallets, 1);
		assert_eq!(
			app.auth.resolve_merged(pair.absorbed_id).await.unwrap(),
			pair.absorbed_id
		);
	}
}

#[tokio::test]
async fn bringing_the_data_needs_the_app_to_cover_every_table() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		// `balances` is `custom`, so without the app's `OnMerge` only the
		// login can move.
		let app = app(&db, false);
		let mut pair = pair(&app, &db).await;
		let offer = pair.survivor.offer(&app).await.unwrap();
		assert_eq!(offer.modes, [MergeMode::MoveLogin]);
		let refused = pair
			.survivor
			.post(
				&app,
				"/auth/merge",
				json!({ "mode": "bring_data", "confirm": true }),
			)
			.await;
		assert_eq!(json_body(refused).await["error"], "unsupported");

		// The other account's second factor is still needed in either mode.
		let setup = json_body(pair.absorbed.post(&app, "/auth/totp", json!({})).await).await;
		let secret = setup["secret"].as_str().unwrap().to_owned();
		pair.absorbed
			.post(
				&app,
				"/auth/totp/confirm",
				json!({ "code": totp::totp(&secret, 0) }),
			)
			.await;
		let without = pair
			.survivor
			.post(
				&app,
				"/auth/merge",
				json!({ "mode": "move_login", "confirm": true }),
			)
			.await;
		assert_eq!(json_body(without).await["error"], "step_up_required");
		let wrong = pair
			.survivor
			.post(
				&app,
				"/auth/merge",
				json!({ "mode": "move_login", "confirm": true, "code": "000000" }),
			)
			.await;
		assert_eq!(json_body(wrong).await["error"], "invalid_code");
		let merged = pair
			.survivor
			.post(
				&app,
				"/auth/merge",
				json!({ "mode": "move_login", "confirm": true, "code": totp::totp(&secret, 1) }),
			)
			.await;

		assert_eq!(merged.status(), StatusCode::NO_CONTENT);

		// "Move only the login": the other account's data went with it.
		assert_eq!(
			snapshot(&db, pair.absorbed_id).await,
			(0, Vec::new(), String::new(), 0, 0)
		);
		assert_eq!(
			snapshot(&db, pair.survivor_id).await,
			(1, vec!["rust".to_owned()], "light".to_owned(), 0, 500)
		);
	}
}

#[tokio::test]
async fn a_duplicate_stops_a_plain_move_and_apps_word_their_own_preview() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app_with(&db, leptoskit_auth::AuthBuilder::on_merge::<Worded>);
		let mut pair = pair(&app, &db).await;
		let offer = pair.survivor.offer(&app).await.unwrap();
		assert_eq!(
			offer.data,
			[MergeData {
				label: "stories".into(),
				count: 2,
				fate: MergeFate::Moves,
			}]
		);

		// Both accounts have the same badge, and `badges` moves plainly.
		let system = db.system("test");

		for user in [pair.survivor_id, pair.absorbed_id] {
			NewBadge {
				user_id: user,
				kind: "early".into(),
			}
			.insert(&system)
			.await
			.unwrap();
		}

		let before = (
			snapshot(&db, pair.survivor_id).await,
			snapshot(&db, pair.absorbed_id).await,
		);
		let refused = pair
			.survivor
			.post(
				&app,
				"/auth/merge",
				json!({ "mode": "bring_data", "confirm": true }),
			)
			.await;
		assert_eq!(refused.status(), StatusCode::CONFLICT);
		let body = json_body(refused).await;
		assert_eq!(body["error"], "merge_conflict");
		assert!(
			!body["message"].as_str().unwrap().contains("badges"),
			"the table stays on the server: {body}"
		);
		assert_eq!(
			before,
			(
				snapshot(&db, pair.survivor_id).await,
				snapshot(&db, pair.absorbed_id).await,
			)
		);
		assert!(
			Account::find(&system, pair.absorbed_id)
				.await
				.unwrap()
				.is_some()
		);

		// Without the duplicate, the other account's profile is left as it
		// was: this app copies nothing from it.
		Badge::query()
			.filter(|badge| badge.user_id.eq(pair.absorbed_id))
			.delete(&system)
			.await
			.unwrap();
		assert_eq!(
			pair.survivor.link(&app, &pair.wallet).await["outcome"],
			"merge"
		);
		let merged = pair
			.survivor
			.post(
				&app,
				"/auth/merge",
				json!({ "mode": "bring_data", "confirm": true }),
			)
			.await;
		assert_eq!(merged.status(), StatusCode::NO_CONTENT);
		let me = User::get(&system, pair.survivor_id).await.unwrap();
		assert_eq!(me.bio, None);
		assert_eq!(snapshot(&db, pair.survivor_id).await.0, 3);
	}
}
