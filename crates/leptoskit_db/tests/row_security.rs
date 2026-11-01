//! Row-level security on Postgres: the policies the database layer applies,
//! rendered as `CREATE POLICY` and enforced by the database for every
//! statement, raw SQL included, with results identical to the database
//! layer's own.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use std::borrow::Cow;

use leptoskit_db::AuditEntry;
use leptoskit_db::AuditOp;
use leptoskit_db::AuditReader;
use leptoskit_db::Connection;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Dialect;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::ListSetting;
use leptoskit_db::Migration;
use leptoskit_db::Setting;
use leptoskit_db::Statement;
use leptoskit_db::Timestamp;
use leptoskit_db::Value;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db::view;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest::test_runner::TestCaseError;
use proptest::test_runner::TestRunner;
use support::postgres::PostgresCluster;

/// The writer a request acts for.
pub struct Viewer(pub Id<Writer>);

impl Setting for Viewer {
	type Value = Id<Writer>;

	const NAME: &'static str = "viewer";

	fn value(&self) -> Id<Writer> {
		self.0
	}
}

leptoskit_db::register_setting!(Viewer);

/// Someone who writes. Anyone reads writers; each changes only their own,
/// and the audit log records it.
#[table(name = "writers", audit)]
#[policy(select, |_, _| Expr::always())]
#[policy(insert, update, delete, |writer, cx| writer.id.eq(cx.setting::<Viewer>()))]
pub struct Writer {
	pub id: Id<Self>,
	pub name: String,
}

/// A post: anyone reads a published one, its writer and the writers it's
/// shared with read it otherwise, only its writer changes it, and nobody
/// changes a locked one.
#[table(name = "posts")]
#[policy(select, |post, cx| post.published.eq(true)
	| post.writer_id.eq(cx.setting::<Viewer>())
	| cx.exists(Share::query().filter(|share| share.post_id.eq(post.id) & share.writer_id.eq(cx.setting::<Viewer>()))))]
#[policy(insert, update, delete, |post, cx| post.writer_id.eq(cx.setting::<Viewer>()))]
#[policy(deny update, |post, _| post.locked.eq(true))]
pub struct Post {
	pub id: Id<Self>,
	pub writer_id: Id<Writer>,
	pub title: String,
	pub published: bool,
	pub locked: bool,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// A post shared with a writer, which only that writer sees.
#[table(name = "shares")]
#[policy(select, |share, cx| share.writer_id.eq(cx.setting::<Viewer>()))]
#[policy(insert, delete, |_, _| Expr::always())]
pub struct Share {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub post_id: Id<Post>,
	pub writer_id: Id<Writer>,
}

/// The writers whose notes a request reads: a list setting.
pub struct Desk(pub Vec<Id<Writer>>);

impl ListSetting for Desk {
	type Item = Id<Writer>;

	const NAME: &'static str = "desk";

	fn values(&self) -> Vec<Id<Writer>> {
		self.0.clone()
	}
}

leptoskit_db::register_list_setting!(Desk);

/// A note about a writer: a request reads the notes of the writers on its
/// desk, and, with a desk, open notes about anyone else. Without a desk,
/// even the negated half matches nothing.
#[table(name = "notes")]
#[policy(select, |note, cx| note.writer_id.in_list(cx.setting_list::<Desk>())
	| (!note.writer_id.in_list(cx.setting_list::<Desk>()) & note.open.eq(true)))]
pub struct Note {
	pub id: Id<Self>,
	pub writer_id: Id<Writer>,
	pub text: String,
	pub open: bool,
}

/// Every post's title, which the posts' policies filter too: Postgres reads
/// `posts` as whoever reads the view.
#[view(name = "titles", sql = "SELECT id, title FROM posts")]
pub struct Title {
	pub id: Id<Post>,
	pub title: String,
}

/// How many posts there were at the last refresh: all of them, whoever
/// refreshed it.
#[view(name = "post_totals", materialized, sql = "SELECT count(*) FROM posts")]
pub struct PostTotal {
	pub posts: i64,
}

/// A role that owns the database but isn't a superuser: Postgres exempts
/// superusers from row-level security, forced or not.
const ROLE: &str = "lk_app";

/// An empty database owned by [`ROLE`], as a URL that connects as it.
async fn owned_database(cluster: &PostgresCluster) -> String {
	let url = cluster.database();
	let name = url
		.rsplit('/')
		.next()
		.and_then(|tail| tail.split('?').next())
		.unwrap()
		.to_string();
	let admin = Db::connect(&url).await.unwrap();
	admin
		.run_script(&format!(
			"DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = '{ROLE}') THEN \
			 CREATE ROLE {ROLE} LOGIN; END IF; END $$; ALTER DATABASE {name} OWNER TO {ROLE}; \
			 ALTER SCHEMA public OWNER TO {ROLE}"
		))
		.await
		.unwrap();
	url.replacen("postgres@", &format!("{ROLE}@"), 1)
}

async fn create(db: &Db, schema: &Schema) {
	let empty = Schema::empty(schema.dialect);
	let plan = leptoskit_db_schema::diff(&empty, schema, &mut RequireHints).unwrap();
	let migration = leptoskit_db_schema::render(&plan, &empty, schema);
	db.run_script(&migration.sql()).await.unwrap();
}

/// A database whose policies Postgres enforces too, and the handle that
/// tells it who asks.
async fn secured(cluster: &PostgresCluster) -> Db {
	let db = Db::connect(&owned_database(cluster).await).await.unwrap();
	let schema = leptoskit_db::schema_with_row_security(Dialect::Postgres).unwrap();
	create(&db, &schema).await;
	db.with_row_security().unwrap()
}

/// A database whose policies only the database layer enforces.
async fn plain(cluster: &PostgresCluster) -> Db {
	let db = Db::connect(&owned_database(cluster).await).await.unwrap();
	create(&db, &leptoskit_db::schema(Dialect::Postgres).unwrap()).await;
	db
}

fn as_writer(db: &Db, writer: Id<Writer>) -> Db {
	db.with_context(Context::new().with(Viewer(writer)))
}

/// Raw SQL's rows, each as text.
async fn raw(connection: &impl Connection, sql: &str) -> Vec<String> {
	let rows = connection
		.fetch(Statement {
			sql: sql.to_string(),
			args: Vec::new(),
		})
		.await
		.unwrap();
	rows.into_iter()
		.map(|row| {
			match row.into_iter().next() {
				Some(Value::Text(text)) => text,
				Some(Value::Int(count)) => count.to_string(),
				other => panic!("unexpected {other:?}"),
			}
		})
		.collect()
}

async fn raw_execute(connection: &impl Connection, sql: &str) -> Result<u64, DbError> {
	connection
		.execute(Statement {
			sql: sql.to_string(),
			args: Vec::new(),
		})
		.await
}

#[test]
fn rules_render_as_policies() {
	let schema = leptoskit_db::schema_with_row_security(Dialect::Postgres).unwrap();
	let rendered: Vec<String> = schema
		.tables
		.iter()
		.flat_map(|table| {
			table
				.row_security
				.iter()
				.flat_map(|security| &security.policies)
				.map(move |policy| {
					format!(
						"{} {}\n  using: {}\n  check: {}",
						table.name,
						policy.command.keyword(),
						policy.using.as_deref().unwrap_or("-"),
						policy.check.as_deref().unwrap_or("-")
					)
				})
		})
		.collect();
	insta::assert_snapshot!(rendered.join("\n"));

	// The plain schema has none, and SQLite can't have any.
	let plain = leptoskit_db::schema(Dialect::Postgres).unwrap();
	assert!(
		plain
			.tables
			.iter()
			.all(|table| table.row_security.is_none())
	);
	let error = leptoskit_db::schema_with_row_security(Dialect::Sqlite)
		.unwrap_err()
		.to_string();
	assert!(
		error.contains("row-level security is Postgres's"),
		"{error}"
	);
}

#[tokio::test]
async fn sqlite_has_no_row_level_security() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	assert!(!db.has_row_security());
	assert!(matches!(
		db.with_row_security(),
		Err(DbError::Connection(_))
	));
}

/// Two writers' posts: Ada's published and draft ones, and Grace's two
/// drafts, one shared with Ada and one locked.
async fn seed(db: &Db) -> (Id<Writer>, Id<Writer>) {
	let system = db.system("seeding");
	let ada = NewWriter { name: "Ada".into() }
		.insert(&system)
		.await
		.unwrap()
		.id;
	let grace = NewWriter {
		name: "Grace".into(),
	}
	.insert(&system)
	.await
	.unwrap()
	.id;

	for (writer, title, published, locked) in [
		(ada, "ada-published", true, false),
		(ada, "ada-draft", false, false),
		(grace, "grace-shared", false, false),
		(grace, "grace-locked", false, true),
	] {
		let post = NewPost {
			writer_id: writer,
			title: title.into(),
			published,
			locked,
		}
		.insert(&system)
		.await
		.unwrap();

		if title == "grace-shared" {
			NewShare {
				post_id: post.id,
				writer_id: ada,
			}
			.insert(&system)
			.await
			.unwrap();
		}
	}

	(ada, grace)
}

#[tokio::test]
async fn raw_sql_is_filtered_by_the_database() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let db = secured(&cluster).await;
	let (ada, grace) = seed(&db).await;
	let titles = "SELECT title FROM posts ORDER BY title";

	// Raw SQL through each request's handle sees what its policy allows.
	assert_eq!(
		raw(&as_writer(&db, ada), titles).await,
		["ada-draft", "ada-published", "grace-shared"]
	);
	assert_eq!(
		raw(&as_writer(&db, grace), titles).await,
		["ada-published", "grace-locked", "grace-shared"]
	);
	assert_eq!(
		raw(&db.with_context(Context::new()), titles).await,
		["ada-published"]
	);
	assert_eq!(raw(&db.system("auditing"), titles).await.len(), 4);
	assert!(
		raw(&as_writer(&db, ada), titles)
			.await
			.iter()
			.all(|title| title != "grace-locked")
	);
	// Shares are Ada's alone, even read directly.
	assert_eq!(
		raw(&as_writer(&db, grace), "SELECT count(*) FROM shares").await,
		["0"]
	);

	// So does a transaction's, for each of its statements.
	let tx = as_writer(&db, grace).begin().await.unwrap();
	assert_eq!(raw(&tx, "SELECT count(*) FROM posts").await, ["3"]);
	tx.rollback().await.unwrap();

	// Writes the policy doesn't allow change nothing, or fail.
	let as_grace = as_writer(&db, grace);
	assert_eq!(
		raw_execute(
			&as_grace,
			"UPDATE posts SET title = 'taken' WHERE title LIKE 'ada%'"
		)
		.await
		.unwrap(),
		0
	);
	// Grace may delete her locked post, so the update policy reaches it, as
	// the database layer's soft delete must; but only to soft-delete it.
	let locked = raw_execute(
		&as_grace,
		"UPDATE posts SET title = 'edited' WHERE title = 'grace-locked'",
	)
	.await
	.unwrap_err();
	assert!(locked.is_forbidden(), "{locked:?}");
	assert_eq!(
		raw_execute(
			&as_grace,
			"UPDATE posts SET deleted_at = now() WHERE title = 'grace-locked'"
		)
		.await
		.unwrap(),
		1
	);
	assert_eq!(
		raw_execute(&as_grace, "DELETE FROM posts WHERE title = 'ada-published'")
			.await
			.unwrap(),
		0
	);
	let forged = raw_execute(
		&as_grace,
		&format!(
			"INSERT INTO posts (id, writer_id, title, published, locked) VALUES \
			 (gen_random_uuid(), '{}', 'forged', true, false)",
			ada.uuid()
		),
	)
	.await;
	let message = forged.unwrap_err().to_string();
	assert!(message.contains("row-level security"), "{message}");
	assert_eq!(
		raw_execute(
			&as_grace,
			"UPDATE posts SET title = 'grace-renamed' WHERE title = 'grace-shared'"
		)
		.await
		.unwrap(),
		1,
		"what the policy allows still works"
	);

	// A connection that says nothing, as any other client of the database,
	// sees only what a signed-out request would.
	let outsider = Db::connect(&db_url(&db, &cluster).await).await.unwrap();
	assert_eq!(raw(&outsider, titles).await, ["ada-published"]);
	assert_eq!(
		raw_execute(&outsider, "DELETE FROM posts").await.unwrap(),
		0
	);
	assert_eq!(
		raw(&db.system("counting"), "SELECT count(*) FROM posts").await,
		["4"]
	);
}

/// What a save and an upsert the rules refuse fail with, for Grace.
async fn refusals(db: &Db) -> Vec<String> {
	let (ada, grace) = seed(db).await;
	let as_grace = as_writer(db, grace);
	let mut locked = Post::query()
		.filter(|post| post.title.eq("grace-locked".to_string()))
		.one(&db.system("finding"))
		.await
		.unwrap();
	locked.title = "unlocked".into();
	let saved = locked.save(&as_grace).await.unwrap_err();
	let forged = NewPost {
		writer_id: ada,
		title: "forged".into(),
		published: true,
		locked: false,
	}
	.upsert(&as_grace, |post| post.id)
	.await
	.unwrap_err();
	let mut taken = Post::query()
		.filter(|post| post.title.eq("grace-shared".to_string()))
		.one(&db.system("finding"))
		.await
		.unwrap();
	taken.writer_id = ada;
	let given_away = taken.save(&as_grace).await.unwrap_err();

	[saved, forged, given_away]
		.iter()
		.map(|error| {
			assert!(error.is_forbidden(), "{error:?}");
			kind(error)
		})
		.collect()
}

#[tokio::test]
async fn refused_saves_and_upserts_say_which_rule_refused_them() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let with = refusals(&secured(&cluster).await).await;
	let without = refusals(&plain(&cluster).await).await;
	// The database layer checks first, so Postgres's own refusal, which
	// names no rule, never reaches the caller.
	assert!(
		with.iter()
			.all(|refusal| !refusal.contains("row-level security")),
		"{with:?}"
	);
	assert_eq!(with, without);
	assert_eq!(
		with,
		[
			"forbidden: you may not change this `posts` row",
			"forbidden: you may not insert this `posts` row",
			"forbidden: you may not change this `posts` row",
		]
	);
}

/// The URL of the database `db` uses, as its owning role, for a second
/// client that sets nothing.
async fn db_url(db: &Db, cluster: &PostgresCluster) -> String {
	let name = raw(&db.system("naming"), "SELECT current_database()::text").await;
	let url = cluster.database();
	let (base, _) = url.rsplit_once('/').unwrap();
	format!(
		"{}/{}?sslmode=disable",
		base.replacen("postgres@", &format!("{ROLE}@"), 1),
		name[0]
	)
}

#[tokio::test]
async fn migrations_change_every_row_whatever_the_policies() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let db = secured(&cluster).await;
	seed(&db).await;
	let sql = "UPDATE posts SET title = upper(title)";
	let migration = Migration {
		name: Cow::Borrowed("20261030T0000_shout"),
		sql: Cow::Borrowed(sql),
		checksum: Cow::Owned(leptoskit_db::checksum(sql)),
	};
	let applied = db.migrate(&[migration]).await.unwrap();
	assert_eq!(applied, ["20261030T0000_shout"]);
	assert_eq!(
		raw(
			&db.system("checking"),
			"SELECT count(*) FROM posts WHERE title = upper(title)"
		)
		.await,
		["4"]
	);
}

#[tokio::test]
async fn views_read_their_tables_as_their_reader() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let db = secured(&cluster).await;
	let (ada, grace) = seed(&db).await;
	// A superuser owning the view would read every post, were it read as
	// its owner.
	let owner = db_url(&db, &cluster)
		.await
		.replacen(&format!("{ROLE}@"), "postgres@", 1);
	Db::connect(&owner)
		.await
		.unwrap()
		.run_script(&format!(
			"ALTER VIEW titles OWNER TO postgres; GRANT SELECT ON titles TO {ROLE}"
		))
		.await
		.unwrap();

	let titles = "SELECT title FROM titles ORDER BY title";
	assert_eq!(
		raw(&as_writer(&db, ada), titles).await,
		["ada-draft", "ada-published", "grace-shared"]
	);
	assert_eq!(
		raw(&db.with_context(Context::new()), titles).await,
		["ada-published"]
	);
	assert_eq!(
		Title::query()
			.all(&as_writer(&db, grace))
			.await
			.unwrap()
			.len(),
		3
	);

	// A materialized view keeps every row, whoever refreshes it, and the
	// refresher's own reads stay filtered afterwards.
	let tx = as_writer(&db, ada).begin().await.unwrap();
	PostTotal::refresh(&tx).await.unwrap();
	assert_eq!(raw(&tx, "SELECT posts FROM post_totals").await, ["4"]);
	assert_eq!(raw(&tx, "SELECT count(*) FROM posts").await, ["3"]);
	tx.commit().await.unwrap();
	PostTotal::refresh(&db.with_context(Context::new()))
		.await
		.unwrap();
	assert_eq!(
		PostTotal::query()
			.all(&db.system("checking"))
			.await
			.unwrap()[0]
			.posts,
		4
	);
}

#[tokio::test]
async fn list_settings_read_the_same_with_and_without_row_security() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let secured = secured(&cluster).await;
	let plain = plain(&cluster).await;

	for (db, rls) in [(&secured, true), (&plain, false)] {
		let (ada, grace) = seed(db).await;
		for (writer, text, open) in [
			(ada, "ada-closed", false),
			(ada, "ada-open", true),
			(grace, "grace-closed", false),
			(grace, "grace-open", true),
		] {
			NewNote {
				writer_id: writer,
				text: text.into(),
				open,
			}
			.insert(&db.system("seeding"))
			.await
			.unwrap();
		}

		let texts = async |context: Context| {
			let mut texts: Vec<String> = Note::query()
				.all(&db.with_context(context))
				.await
				.unwrap()
				.into_iter()
				.map(|note| note.text)
				.collect();
			texts.sort();
			texts
		};
		let desk = |writers: Vec<Id<Writer>>| Context::new().with(Desk(writers));

		assert_eq!(
			texts(desk(vec![ada])).await,
			["ada-closed", "ada-open", "grace-open"]
		);
		assert_eq!(
			texts(desk(vec![ada, grace])).await,
			["ada-closed", "ada-open", "grace-closed", "grace-open"]
		);
		// An empty desk holds nobody, so the negated half opens the open
		// notes; no desk at all matches nothing.
		assert_eq!(texts(desk(Vec::new())).await, ["ada-open", "grace-open"]);
		assert_eq!(texts(Context::new()).await, Vec::<String>::new());

		if rls {
			let sql = "SELECT text FROM notes ORDER BY text";
			assert_eq!(
				raw(&secured.with_context(desk(vec![grace])), sql).await,
				["ada-open", "grace-closed", "grace-open"]
			);
			assert_eq!(
				raw(&secured.with_context(Context::new()), sql).await,
				Vec::<String>::new()
			);
		}
	}
}

#[tokio::test]
async fn the_audit_log_opens_only_to_readers() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let db = secured(&cluster).await;
	let (ada, _) = seed(&db).await;
	let as_ada = as_writer(&db, ada);

	// A request's change is recorded as that request.
	let mut writer = Writer::query()
		.filter(|writer| writer.id.eq(ada))
		.one(&as_ada)
		.await
		.unwrap();
	writer.name = "Ada L.".into();
	writer.save(&as_ada).await.unwrap();

	let log = "SELECT count(*) FROM _leptoskit_audit";
	assert_eq!(raw(&as_ada, log).await, ["0"]);
	assert!(AuditEntry::query().all(&as_ada).await.unwrap().is_empty());

	// Postgres sees that a request holds a reader, not what its rule
	// matches: raw SQL reads every entry, the database layer what the rule
	// allows.
	let reading = |reader| db.with_context(Context::new().with(Viewer(ada)).with(reader));
	let updates = reading(AuditReader::matching(|entry, _| {
		entry.op.eq(AuditOp::Update)
	}));
	assert_eq!(raw(&updates, log).await, ["3"]);
	assert_eq!(AuditEntry::query().all(&updates).await.unwrap().len(), 1);

	// Nobody but `system` changes or removes an entry.
	let everything = reading(AuditReader::everything());
	assert_eq!(
		raw_execute(&everything, "UPDATE _leptoskit_audit SET reason = 'forged'")
			.await
			.unwrap(),
		0
	);
	assert_eq!(
		raw_execute(&everything, "DELETE FROM _leptoskit_audit")
			.await
			.unwrap(),
		0
	);
	AuditEntry::query()
		.destroy(&db.system("retention"))
		.await
		.unwrap();
	assert_eq!(raw(&everything, log).await, ["0"]);
}

/// What a property does: writers, their posts, and which are shared.
#[derive(Clone, Debug)]
struct Spec {
	posts: Vec<(usize, bool, bool, bool)>,
	shares: Vec<(usize, usize)>,
	viewer: Option<usize>,
	/// Which writer's post the request tries to insert.
	inserts_for: usize,
}

const WRITERS: usize = 3;

fn spec() -> impl Strategy<Value = Spec> {
	(
		proptest::collection::vec(
			(
				0..WRITERS,
				proptest::bool::ANY,
				proptest::bool::weighted(0.2),
				proptest::bool::weighted(0.2),
			),
			0..7,
		),
		proptest::collection::vec((0..7_usize, 0..WRITERS), 0..4),
		proptest::option::of(0..WRITERS),
		0..WRITERS,
	)
		.prop_map(|(posts, shares, viewer, inserts_for)| {
			Spec {
				posts,
				shares,
				viewer,
				inserts_for,
			}
		})
}

/// What a request sees and does, in a form both databases agree on.
#[derive(Debug, PartialEq)]
struct Outcome {
	visible: Vec<String>,
	count: u64,
	shares: u64,
	edited: Result<u64, String>,
	after_edit: Vec<String>,
	inserted: Result<String, String>,
	deleted: Result<u64, String>,
	remaining: u64,
}

fn kind(error: &DbError) -> String {
	match error {
		DbError::Forbidden { message, .. } => format!("forbidden: {message}"),
		DbError::NotFound { .. } => "not found".into(),
		other => format!("other: {other}"),
	}
}

async fn outcome(db: &Db, spec: &Spec) -> Outcome {
	let system = db.system("resetting the property");
	Share::query().destroy(&system).await.unwrap();
	Post::query().destroy(&system).await.unwrap();
	Writer::query().destroy(&system).await.unwrap();
	let mut writers = Vec::new();

	for index in 0..WRITERS {
		writers.push(
			NewWriter {
				name: format!("w{index}"),
			}
			.insert(&system)
			.await
			.unwrap()
			.id,
		);
	}

	let mut posts = Vec::new();

	for (index, (writer, published, locked, deleted)) in spec.posts.iter().enumerate() {
		let mut post = NewPost {
			writer_id: writers[*writer],
			title: format!("p{index}"),
			published: *published,
			locked: *locked,
		}
		.insert(&system)
		.await
		.unwrap();

		if *deleted {
			post.delete(&system).await.unwrap();
		}

		posts.push(post.id);
	}

	for (post, writer) in &spec.shares {
		if let Some(post) = posts.get(*post) {
			NewShare {
				post_id: *post,
				writer_id: writers[*writer],
			}
			.insert(&system)
			.await
			.unwrap();
		}
	}

	let request = match spec.viewer {
		Some(viewer) => as_writer(db, writers[viewer]),
		None => db.with_context(Context::new()),
	};
	let titles = async |request: &Db| {
		let mut titles: Vec<String> = Post::query()
			.all(request)
			.await
			.unwrap()
			.into_iter()
			.map(|post| post.title)
			.collect();
		titles.sort();
		titles
	};
	let visible = titles(&request).await;
	let count = Post::query().count(&request).await.unwrap();
	let shares = Share::query().count(&request).await.unwrap();
	let edited = Post::update()
		.filter(|post| post.published.eq(false))
		.set(|post| post.title, "edited".to_string())
		.run(&request)
		.await
		.map_err(|error| kind(&error));
	let after_edit = titles(&request).await;
	let inserted = NewPost {
		writer_id: writers[spec.inserts_for],
		title: "new".into(),
		published: false,
		locked: false,
	}
	.insert(&request)
	.await
	.map(|post| post.title)
	.map_err(|error| kind(&error));
	let deleted = Post::query()
		.filter(|post| post.published.eq(true))
		.delete(&request)
		.await
		.map_err(|error| kind(&error));
	let remaining = Post::query().count(&system).await.unwrap();

	Outcome {
		visible,
		count,
		shares,
		edited,
		after_edit,
		inserted,
		deleted,
		remaining,
	}
}

async fn agrees(secured: &Db, plain: &Db, spec: Spec) -> Result<(), TestCaseError> {
	let with = outcome(secured, &spec).await;
	let without = outcome(plain, &spec).await;
	prop_assert_eq!(with, without);
	Ok(())
}

#[test]
fn results_are_the_same_with_and_without_row_level_security() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let (secured, plain) =
		runtime.block_on(async { (secured(&cluster).await, plain(&cluster).await) });
	let mut runner = TestRunner::new(Config {
		cases: 48,
		failure_persistence: None,
		..Config::default()
	});
	let outcome = runner.run(&spec(), |spec| {
		runtime.block_on(agrees(&secured, &plain, spec))
	});

	if let Err(error) = outcome {
		panic!("{error}");
	}
}
