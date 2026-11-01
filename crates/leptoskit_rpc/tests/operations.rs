//! Queries and mutations, end to end on both databases: the HTTP routes,
//! policies as the caller, receipts that make a repeated call run once,
//! serializable retries, `after_commit`, and calls from server code.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

#[path = "../../leptoskit_db/tests/support/postgres.rs"]
mod postgres;

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use http_body_util::BodyExt;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_query::BoxFuture;
use leptoskit_rpc as rpc;
use leptoskit_rpc::Operation;
use leptoskit_rpc::api;
use leptoskit_rpc::server::Identify;
use leptoskit_rpc::server::Identity;
use leptoskit_rpc::server::Server;
use postgres::PostgresCluster;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt;

/// Who is signed in, as policies read it.
#[derive(Clone, Debug)]
pub struct Me(pub String);

/// A note someone keeps.
#[table(name = "notes")]
#[policy(all, |note, cx| cx.global(|me: &Me| note.owner.eq(me.0.clone())))]
pub struct Note {
	pub id: Id<Self>,
	#[db(max_len = 64)]
	pub owner: String,
	#[db(max_len = 200)]
	pub title: String,
}

/// A counter two mutations race on.
#[table(name = "counters")]
pub struct Counter {
	pub id: Id<Self>,
	#[db(max_len = 64)]
	pub name: String,
	pub value: i64,
}

/// The caller's notes, newest last.
///
/// # Arguments
///
/// * `starting` - Only titles beginning with this.
#[api(tag = "notes")]
#[rpc::query]
pub async fn notes(
	cx: &rpc::QueryCx,
	#[serde(default)] starting: Option<String>,
) -> rpc::Result<Vec<String>> {
	let notes = Note::query().all(cx.db()).await?;
	Ok(notes
		.into_iter()
		.map(|note| note.title)
		.filter(|title| {
			starting
				.as_deref()
				.is_none_or(|prefix| title.starts_with(prefix))
		})
		.collect())
}

/// What `after_commit` saw, in order.
static COMMITTED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Keeps a note for the signed-in person.
#[api]
#[rpc::mutation]
pub async fn add_note(cx: &rpc::MutationCx, title: String) -> rpc::Result<Id<Note>> {
	let me = cx.require::<Me>()?;
	if title.is_empty() {
		return Err(rpc::Error::invalid("a note needs a title"));
	}
	let note = NewNote {
		owner: me.0.clone(),
		title: title.clone(),
	}
	.insert(cx.tx())
	.await?;
	cx.after_commit(async move {
		COMMITTED.lock().unwrap().push(title);
	});
	Ok(note.id)
}

/// Fails after writing, so nothing it wrote commits.
#[rpc::mutation]
async fn add_then_fail(cx: &rpc::MutationCx, title: String) -> rpc::Result<()> {
	let me = cx.require::<Me>()?;
	NewNote {
		owner: me.0.clone(),
		title: title.clone(),
	}
	.insert(cx.tx())
	.await?;
	cx.after_commit(async move {
		COMMITTED.lock().unwrap().push(format!("never {title}"));
	});
	Err(rpc::Error::forbidden("not today"))
}

/// Adds two notes through another mutation, in one transaction.
#[rpc::mutation(name = "add_two")]
async fn add_pair(cx: &rpc::MutationCx, first: String, second: String) -> rpc::Result<u32> {
	rpc::call(cx, AddNote { title: first }).await?;
	rpc::call(cx, AddNote { title: second }).await?;
	Ok(2)
}

/// How many runs `bump` has started, retries included.
static BUMP_RUNS: AtomicU32 = AtomicU32::new(0);

/// Reads a counter, waits, and writes it back one higher: two of these at
/// once conflict, and serializable isolation makes one run again.
#[rpc::mutation]
async fn bump(cx: &rpc::MutationCx, name: String) -> rpc::Result<i64> {
	BUMP_RUNS.fetch_add(1, Ordering::SeqCst);
	let mut counter = Counter::query()
		.filter(|counter| counter.name.eq(name.clone()))
		.one(cx.tx())
		.await?;
	tokio::time::sleep(Duration::from_millis(100)).await;
	counter.value += 1;
	counter.save(cx.tx()).await?;
	Ok(counter.value)
}

/// `bump`, without retries.
#[rpc::mutation(retries = 0)]
async fn bump_once(cx: &rpc::MutationCx, name: String) -> rpc::Result<i64> {
	let mut counter = Counter::query()
		.filter(|counter| counter.name.eq(name.clone()))
		.one(cx.tx())
		.await?;
	tokio::time::sleep(Duration::from_millis(100)).await;
	counter.value += 1;
	counter.save(cx.tx()).await?;
	Ok(counter.value)
}

/// How many notes the caller has, through the `notes` query.
#[rpc::query]
async fn note_count(cx: &rpc::QueryCx) -> rpc::Result<usize> {
	Ok(rpc::call(cx, Notes { starting: None }).await?.len())
}

/// Trusts an `x-user` header: enough to tell callers apart in a test.
struct Header;

impl Identify for Header {
	fn identify<'a>(
		&'a self,
		request: &'a http::request::Parts,
	) -> BoxFuture<'a, rpc::Result<Identity>> {
		Box::pin(async move {
			let Some(user) = request.headers.get("x-user") else {
				return Ok(Identity::anonymous());
			};
			let user = user
				.to_str()
				.map_err(|_| rpc::Error::unauthorized("bad user"))?;
			if user == "banned" {
				return Err(rpc::Error::unauthorized("banned"));
			}
			Ok(Identity::new(Context::new().with(Me(user.to_owned()))).with_caller(user))
		})
	}
}

struct Call<'a> {
	name: &'a str,
	user: Option<&'a str>,
	key: Option<&'a str>,
	body: String,
}

impl<'a> Call<'a> {
	fn new(name: &'a str, body: &Value) -> Self {
		Self {
			name,
			user: None,
			key: None,
			body: body.to_string(),
		}
	}

	fn user(mut self, user: &'a str) -> Self {
		self.user = Some(user);
		self
	}

	fn key(mut self, key: &'a str) -> Self {
		self.key = Some(key);
		self
	}

	fn raw(mut self, body: &str) -> Self {
		body.clone_into(&mut self.body);
		self
	}

	async fn send(self, router: &Router) -> (StatusCode, String) {
		let mut request =
			Request::post(format!("/api/{}", self.name)).header("content-type", "application/json");
		if let Some(user) = self.user {
			request = request.header("x-user", user);
		}
		if let Some(key) = self.key {
			request = request.header("idempotency-key", key);
		}
		let response = router
			.clone()
			.oneshot(request.body(Body::from(self.body)).unwrap())
			.await
			.unwrap();
		let status = response.status();
		let bytes = response.into_body().collect().await.unwrap().to_bytes();
		(status, String::from_utf8(bytes.to_vec()).unwrap())
	}
}

async fn sqlite() -> Db {
	leptoskit_db::testing::sqlite().await
}

async fn postgres(cluster: &PostgresCluster) -> Db {
	let db = Db::connect(&cluster.database()).await.unwrap();
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	db
}

fn as_user(db: &Db, user: &str) -> Db {
	db.with_context(Context::new().with(Me(user.to_owned())))
}

async fn routes_answer_as_the_caller(db: &Db) {
	let router = Server::new(db.clone()).identify(Header).router::<()>();

	let (status, body) = Call::new("add_note", &json!({ "title": "milk" }))
		.user("ada")
		.send(&router)
		.await;
	assert_eq!(status, StatusCode::OK, "{body}");
	assert!(serde_json::from_str::<Id<Note>>(&body).is_ok(), "{body}");
	Call::new("add_note", &json!({ "title": "mangoes" }))
		.user("ada")
		.send(&router)
		.await;
	Call::new("add_note", &json!({ "title": "bread" }))
		.user("grace")
		.send(&router)
		.await;

	// Policies read the caller: each sees their own.
	let (status, body) = Call::new("notes", &json!({}))
		.user("ada")
		.send(&router)
		.await;
	assert_eq!(status, StatusCode::OK);
	let mut titles: Vec<String> = serde_json::from_str(&body).unwrap();
	titles.sort();
	assert_eq!(titles, ["mangoes", "milk"]);
	let (_, body) = Call::new("notes", &json!({ "starting": "b" }))
		.user("grace")
		.send(&router)
		.await;
	assert_eq!(body, r#"["bread"]"#);
	// An empty body is no arguments, and `starting` may be left out.
	let (status, body) = Call::new("notes", &json!({})).raw("").send(&router).await;
	assert_eq!((status, body.as_str()), (StatusCode::OK, "[]"));
	let (_, body) = Call::new("note_count", &json!({}))
		.user("ada")
		.send(&router)
		.await;
	assert_eq!(body, "2");
}

async fn failures_arrive_classified(db: &Db) {
	let router = Server::new(db.clone()).identify(Header).router::<()>();

	// Failures arrive classified, as a server function's would.
	let (status, body) = Call::new("add_note", &json!({ "title": "" }))
		.user("ada")
		.send(&router)
		.await;
	assert_eq!(
		(status, body.as_str()),
		(StatusCode::BAD_REQUEST, "Args|a note needs a title")
	);
	let (status, body) = Call::new("add_note", &json!({ "title": "x" }))
		.send(&router)
		.await;
	assert_eq!(status, StatusCode::UNAUTHORIZED);
	assert!(body.starts_with("ServerError|"), "{body}");
	let (status, body) = Call::new("add_note", &json!({ "title": 7 }))
		.user("ada")
		.send(&router)
		.await;
	assert_eq!(status, StatusCode::BAD_REQUEST);
	assert!(
		body.starts_with("Args|the arguments of `add_note` don't fit"),
		"{body}"
	);
	let (status, _) = Call::new("add_note", &json!({}))
		.raw("[1]")
		.user("ada")
		.send(&router)
		.await;
	assert_eq!(status, StatusCode::BAD_REQUEST);
	let (status, body) = Call::new("notes", &json!({}))
		.user("banned")
		.send(&router)
		.await;
	assert_eq!(
		(status, body.as_str()),
		(StatusCode::UNAUTHORIZED, "ServerError|banned")
	);
}

async fn mutations_commit_all_or_nothing(db: &Db) {
	let router = Server::new(db.clone()).identify(Header).router::<()>();

	// A failed mutation commits nothing, and its after-commit work never runs.
	let (status, body) = Call::new("add_then_fail", &json!({ "title": "ghost" }))
		.user("ada")
		.send(&router)
		.await;
	assert_eq!(
		(status, body.as_str()),
		(StatusCode::FORBIDDEN, "ServerError|not today")
	);
	let (_, body) = Call::new("note_count", &json!({}))
		.user("ada")
		.send(&router)
		.await;
	assert_eq!(body, "2");

	// A mutation can call another in its own transaction, by its own name.
	let (status, body) = Call::new("add_two", &json!({ "first": "a", "second": "b" }))
		.user("grace")
		.send(&router)
		.await;
	assert_eq!((status, body.as_str()), (StatusCode::OK, "2"));
	let (_, body) = Call::new("note_count", &json!({}))
		.user("grace")
		.send(&router)
		.await;
	assert_eq!(body, "3");

	// After-commit work runs in the background, once each.
	tokio::time::sleep(Duration::from_millis(50)).await;
	let committed = COMMITTED.lock().unwrap().clone();
	assert!(committed.contains(&"milk".to_owned()));
	assert!(!committed.iter().any(|title| title.starts_with("never")));
}

async fn a_repeated_call_runs_once(db: &Db) {
	let router = Server::new(db.clone()).identify(Header).router::<()>();
	let first = Call::new("add_note", &json!({ "title": "once" }))
		.user("ada")
		.key("k-1")
		.send(&router)
		.await;
	let second = Call::new("add_note", &json!({ "title": "once" }))
		.user("ada")
		.key("k-1")
		.send(&router)
		.await;
	assert_eq!(first.0, StatusCode::OK);
	assert_eq!(first, second, "the repeat answers with the first result");
	let count = Note::query()
		.filter(|note| note.title.eq("once".to_owned()))
		.count(&as_user(db, "ada"))
		.await
		.unwrap();
	assert_eq!(count, 1);

	// The same key with other arguments is another call, and is refused.
	let (status, body) = Call::new("add_note", &json!({ "title": "twice" }))
		.user("ada")
		.key("k-1")
		.send(&router)
		.await;
	assert_eq!(status, StatusCode::CONFLICT);
	assert!(body.contains("already used"), "{body}");

	// Keys are each caller's own.
	let (status, body) = Call::new("add_note", &json!({ "title": "once" }))
		.user("grace")
		.key("k-1")
		.send(&router)
		.await;
	assert_eq!(status, StatusCode::OK);
	assert_ne!(body, first.1);

	// Anonymous callers' repeats run once too.
	NewCounter {
		name: "anonymous".to_owned(),
		value: 0,
	}
	.insert(db)
	.await
	.unwrap();
	let bump = || {
		Call::new("bump", &json!({ "name": "anonymous" }))
			.key("k-anonymous")
			.send(&router)
	};
	let first = bump().await;
	assert_eq!(first, (StatusCode::OK, "1".to_owned()));
	assert_eq!(bump().await, first);

	// Queries ignore keys, and a key that is too long is refused.
	let (status, _) = Call::new("notes", &json!({}))
		.user("ada")
		.key("k-1")
		.send(&router)
		.await;
	assert_eq!(status, StatusCode::OK);
	let long = "k".repeat(300);
	let (status, _) = Call::new("add_note", &json!({ "title": "long" }))
		.user("ada")
		.key(&long)
		.send(&router)
		.await;
	assert_eq!(status, StatusCode::BAD_REQUEST);
}

async fn receipts_stay_private_and_expire(db: &Db) {
	// Receipts are each caller's: another caller's policy hides them.
	let mine = rpc::server::receipts::Receipt::query()
		.count(&db.with_context(
			Context::new().with(rpc::server::receipts::ReceiptOwner("ada".to_owned())),
		))
		.await
		.unwrap();
	assert_eq!(mine, 1);
	let theirs = rpc::server::receipts::Receipt::query()
		.count(&as_user(db, "nobody"))
		.await
		.unwrap();
	assert_eq!(theirs, 0);
	let anonymous = rpc::server::receipts::Receipt::query()
		.count(
			&db.with_context(Context::new().with(rpc::server::receipts::ReceiptOwner(
				rpc::server::receipts::ANONYMOUS.to_owned(),
			))),
		)
		.await
		.unwrap();
	assert_eq!(anonymous, 1);

	// Pruning drops old receipts, so the call would run again.
	let system = db.system("test");
	tokio::time::sleep(Duration::from_millis(20)).await;
	assert_eq!(
		rpc::server::receipts::prune(&system, Duration::from_secs(3600))
			.await
			.unwrap(),
		0
	);
	assert_eq!(
		rpc::server::receipts::prune(&system, Duration::ZERO)
			.await
			.unwrap(),
		3
	);
}

#[expect(
	clippy::cognitive_complexity,
	reason = "one end-to-end script reads best top to bottom"
)]
async fn server_code_calls_operations(db: &Db) {
	let ada = as_user(db, "ada");
	let id = rpc::call(
		&ada,
		AddNote {
			title: "direct".to_owned(),
		},
	)
	.await
	.unwrap();
	let notes = rpc::call(
		&ada,
		Notes {
			starting: Some("dir".to_owned()),
		},
	)
	.await
	.unwrap();
	assert_eq!(notes, ["direct"]);
	assert!(matches!(
		rpc::call(
			db,
			AddNote {
				title: "nobody".to_owned()
			}
		)
		.await,
		Err(rpc::Error::Unauthorized(_))
	));

	// Run with a request ID, as the routes do.
	let identity = Identity::new(Context::new().with(Me("ada".to_owned()))).with_caller("ada");
	let first = rpc::server::call_as(
		db,
		identity.clone(),
		Some("direct-1"),
		AddNote {
			title: "keyed".to_owned(),
		},
	)
	.await
	.unwrap();
	let again = rpc::server::call_as(
		db,
		identity,
		Some("direct-1"),
		AddNote {
			title: "keyed".to_owned(),
		},
	)
	.await
	.unwrap();
	assert_eq!(first, again);
	assert_ne!(first, id);

	// A query runs as the caller and records what it reads: here, the
	// notes, under their policy for Ada.
	let cx = rpc::QueryCx::new(ada.clone(), Identity::of(&ada));
	assert_eq!(cx.reads(), Vec::new());
	Notes { starting: None }.run(&cx).await.unwrap();
	assert!(cx.get::<Me>().is_some());
	assert!(cx.require::<String>().is_err());
	assert!(format!("{cx:?}").contains("QueryCx"));
	let reads = cx.into_reads();
	assert_eq!(reads.len(), 1);
	assert_eq!(reads[0].table, <Note as leptoskit_db::Table>::table_name());
	assert_eq!(reads[0].kind, leptoskit_db::ReadKind::Rows);
	assert!(reads[0].keys.as_ref().is_some_and(|keys| !keys.is_empty()));

	// Run by name, a query hands its read set to a sink; a query inside
	// it reads into the same set. A mutation adds nothing.
	let registration = rpc::server::registration("note_count").unwrap();
	let sink = rpc::server::ReadSink::new();
	let count = (registration.handler)(rpc::server::Invocation {
		db: ada.clone(),
		identity: Identity::of(&ada),
		request_id: None,
		arguments: json!({}),
		reads: Some(sink.clone()),
	})
	.await
	.unwrap();
	assert!(count.as_u64().is_some_and(|count| count > 0));
	let reads = sink.take();
	assert_eq!(reads.len(), 1);
	assert_eq!(reads[0].table, <Note as leptoskit_db::Table>::table_name());
	assert_eq!(sink.take(), Vec::<leptoskit_db::ReadEntry>::new());
	let registration = rpc::server::registration("add_note").unwrap();
	(registration.handler)(rpc::server::Invocation {
		db: ada.clone(),
		identity: Identity::of(&ada),
		request_id: None,
		arguments: json!({ "title": "sunk" }),
		reads: Some(sink.clone()),
	})
	.await
	.unwrap();
	assert_eq!(sink.take(), Vec::<leptoskit_db::ReadEntry>::new());
}

#[tokio::test]
async fn sqlite_routes_receipts_and_calls() {
	let db = sqlite().await;
	routes_answer_as_the_caller(&db).await;
	failures_arrive_classified(&db).await;
	mutations_commit_all_or_nothing(&db).await;
	let db = sqlite().await;
	a_repeated_call_runs_once(&db).await;
	receipts_stay_private_and_expire(&db).await;
	server_code_calls_operations(&sqlite().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn postgres_routes_receipts_retries_and_calls() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let db = postgres(&cluster).await;
	routes_answer_as_the_caller(&db).await;
	failures_arrive_classified(&db).await;
	mutations_commit_all_or_nothing(&db).await;
	let db = postgres(&cluster).await;
	a_repeated_call_runs_once(&db).await;
	receipts_stay_private_and_expire(&db).await;
	server_code_calls_operations(&postgres(&cluster).await).await;

	// Two bumps at once read the same value; serializable isolation makes
	// one run again, so neither increment is lost.
	let db = postgres(&cluster).await;
	NewCounter {
		name: "c".to_owned(),
		value: 0,
	}
	.insert(&db)
	.await
	.unwrap();
	BUMP_RUNS.store(0, Ordering::SeqCst);
	let bump = || {
		rpc::call(
			&db,
			Bump {
				name: "c".to_owned(),
			},
		)
	};
	let (a, b) = tokio::join!(bump(), bump());
	let mut values = vec![a.unwrap(), b.unwrap()];
	values.sort_unstable();
	assert_eq!(values, [1, 2]);
	assert!(BUMP_RUNS.load(Ordering::SeqCst) >= 3, "one bump ran again");

	// Without retries, the loser gives up.
	let once = || {
		rpc::call(
			&db,
			BumpOnce {
				name: "c".to_owned(),
			},
		)
	};
	let (a, b) = tokio::join!(once(), once());
	let outcomes = [a, b];
	assert!(outcomes.iter().any(Result::is_ok));
	assert!(
		outcomes
			.iter()
			.any(|outcome| matches!(outcome, Err(rpc::Error::Contended))),
		"{outcomes:?}"
	);
}

#[test]
fn published_operations_join_the_spec_with_their_kind() {
	let spec = rpc::spec::collect(&rpc::spec::SpecOptions::new("Notes", "1.0.0")).unwrap();
	let notes = spec
		.functions
		.iter()
		.find(|function| function.name == "notes")
		.unwrap();
	assert_eq!(notes.kind, rpc::spec::ApiKind::Query);
	assert_eq!(
		(notes.method.as_str(), notes.path.as_str()),
		("POST", "/api/notes")
	);
	assert_eq!(notes.struct_name, "Notes");
	assert_eq!(notes.tags, ["notes"]);
	assert_eq!(notes.docs, "The caller's notes, newest last.");
	assert_eq!(notes.params.len(), 1, "the context isn't an argument");
	assert_eq!(notes.params[0].name, "starting");
	assert_eq!(notes.params[0].docs, "Only titles beginning with this.");
	assert!(!notes.params[0].required);
	assert_eq!(notes.input, rpc::spec::HttpInput::Json);

	let add = spec
		.functions
		.iter()
		.find(|function| function.name == "add_note")
		.unwrap();
	assert_eq!(add.kind, rpc::spec::ApiKind::Mutation);
	assert!(add.params[0].required);
	// Without `#[api]` an operation is served, but not published.
	assert!(
		spec.functions
			.iter()
			.all(|function| function.name != "bump")
	);

	// The crate's own operations, such as `rpc.job_status` with a driver,
	// register beside the app's.
	let names: Vec<&str> = rpc::server::registrations()
		.unwrap()
		.iter()
		.map(|registration| registration.name)
		.filter(|name| !name.starts_with("rpc."))
		.collect();
	assert_eq!(
		names,
		[
			"add_note",
			"add_then_fail",
			"add_two",
			"bump",
			"bump_once",
			"note_count",
			"notes"
		]
	);
	assert_eq!(
		rpc::server::registration("add_two").map(|registration| registration.kind),
		Some(rpc::Kind::Mutation)
	);
	assert_eq!(AddPair::NAME, "add_two");
	assert_eq!(<Notes as Operation>::KIND, rpc::Kind::Query);
	assert_eq!(Bump::RETRIES, 5);
	assert_eq!(BumpOnce::RETRIES, 0);
	let _ = Arc::new(Header);
}
