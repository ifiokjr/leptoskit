//! Views against both databases: created by migrations, read like tables
//! under their own policies, refreshed when materialized, and never
//! written.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use leptoskit_db::Connection;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::View;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db::view;
use leptoskit_db_schema::Column;
use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use leptoskit_db_schema::Step;
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest::test_runner::TestCaseError;
use proptest::test_runner::TestRunner;

/// The request's author.
struct Viewer(Id<Author>);

/// Someone who writes.
#[table(name = "authors")]
pub struct Author {
	pub id: Id<Self>,
	pub name: String,
}

/// A post.
#[table(name = "entries")]
pub struct Entry {
	pub id: Id<Self>,
	pub author_id: Id<Author>,
	pub score: i64,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// How much each author has written, as of now.
#[view(
	name = "author_totals",
	public,
	sql = "
	SELECT author_id, count(*) AS entries, max(score) AS best
	FROM entries WHERE deleted_at IS NULL GROUP BY author_id"
)]
#[policy(select, |totals, cx| cx.global(|viewer: &Viewer| totals.author_id.eq(viewer.0)))]
pub struct AuthorTotals {
	pub author_id: Id<Author>,
	pub entries: i64,
	#[db(private)]
	pub best: i64,
}

/// The same, kept until refreshed, with a query for each dialect.
#[view(
	name = "author_snapshots",
	materialized,
	sql(
		postgres = "SELECT author_id, count(*) FROM entries GROUP BY author_id",
		sqlite = "SELECT author_id, count(*) FROM entries GROUP BY author_id",
	)
)]
pub struct AuthorSnapshot {
	pub author_id: Id<Author>,
	pub entries: i64,
}

/// The authors whose best entry scores five or more: a view over a view,
/// named so that creating views by name would create it first.
#[view(
	name = "author_bests",
	sql = "SELECT author_id, best FROM author_totals WHERE best >= 5"
)]
pub struct AuthorBest {
	pub author_id: Id<Author>,
	pub best: i64,
}

/// Entries per author, indexed, so Postgres can refresh it while it's read.
#[view(
	name = "author_counts",
	materialized,
	unique(author_id),
	index(entries),
	sql = "SELECT author_id, count(*) FROM entries GROUP BY author_id"
)]
pub struct AuthorCount {
	pub author_id: Id<Author>,
	pub entries: i64,
}

/// A view's row with the author it links to.
#[derive(leptoskit_db::Shape, Debug, PartialEq)]
#[db(from = AuthorTotals)]
pub struct TotalsCard {
	pub entries: i64,
	#[db(via = author)]
	pub author: AuthorName,
}

#[derive(leptoskit_db::Shape, Debug, PartialEq)]
#[db(from = Author)]
pub struct AuthorName {
	pub name: String,
}

async fn create_schema(db: &Db) -> Schema {
	let dialect = db.dialect();
	let schema = leptoskit_db::schema(dialect).unwrap();
	let empty = Schema::empty(dialect);
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
	let migration = leptoskit_db_schema::render(&plan, &empty, &schema);
	db.run_script(&migration.statements.join(";\n"))
		.await
		.unwrap();
	schema
}

async fn on_both<F, Fut>(check: F)
where
	F: Fn(Db, Schema) -> Fut,
	Fut: Future<Output = ()>,
{
	let db = Db::connect("sqlite::memory:").await.unwrap();
	let schema = create_schema(&db).await;
	check(db, schema).await;

	if let Some(cluster) = support::postgres::PostgresCluster::start() {
		let db = Db::connect(&cluster.database()).await.unwrap();
		let schema = create_schema(&db).await;
		check(db, schema).await;
	}
}

async fn author(db: &Db, name: &str, scores: &[i64]) -> Id<Author> {
	let id = NewAuthor { name: name.into() }.insert(db).await.unwrap().id;

	for score in scores {
		NewEntry {
			author_id: id,
			score: *score,
		}
		.insert(db)
		.await
		.unwrap();
	}

	id
}

fn as_author(db: &Db, id: Id<Author>) -> Db {
	db.with_context(Context::new().with(Viewer(id)))
}

#[tokio::test]
async fn plain_views_are_read_like_tables_under_their_own_policies() {
	on_both(async |db, _| {
		let ada = author(&db, "Ada", &[3, 9]).await;
		let grace = author(&db, "Grace", &[4]).await;
		let system = db.system("reading every total");

		let mut totals = AuthorTotals::query()
			.order_by(|totals| totals.entries.desc())
			.all(&system)
			.await
			.unwrap();
		assert_eq!(totals.len(), 2);
		let first = totals.remove(0);
		assert_eq!((first.author_id, first.entries, first.best), (ada, 2, 9));
		assert_eq!(first.author(&system).await.unwrap().name, "Ada");

		// The view's own policy: each author sees their own totals.
		let mine = AuthorTotals::query()
			.all(&as_author(&db, grace))
			.await
			.unwrap();
		assert_eq!(mine.len(), 1);
		assert_eq!(mine[0].author_id, grace);
		assert_eq!(AuthorTotals::query().count(&db).await.unwrap(), 0);

		// It reads live rows.
		let mut entry = Entry::by_author(grace).one(&db).await.unwrap();
		entry.delete(&db).await.unwrap();
		assert_eq!(AuthorTotals::query().count(&system).await.unwrap(), 1);
		assert!(
			AuthorTotals::query()
				.filter(|t| t.best.gt(5))
				.exists(&system)
				.await
				.unwrap()
		);

		// The public type leaves the private column out.
		let public: PublicAuthorTotals = first.into();
		let json = serde_json::to_value(&public).unwrap();
		assert_eq!(json["entries"], 2);
		assert!(json.get("best").is_none());
	})
	.await;
}

#[tokio::test]
async fn materialized_views_keep_their_rows_until_refreshed() {
	on_both(async |db, _| {
		let ada = author(&db, "Ada", &[1]).await;
		const { assert!(AuthorSnapshot::MATERIALIZED) };
		assert_eq!(AuthorSnapshot::query().count(&db).await.unwrap(), 0);

		AuthorSnapshot::refresh(&db).await.unwrap();
		let snapshot = AuthorSnapshot::query().one(&db).await.unwrap();
		assert_eq!((snapshot.author_id, snapshot.entries), (ada, 1));

		NewEntry {
			author_id: ada,
			score: 2,
		}
		.insert(&db)
		.await
		.unwrap();
		assert_eq!(AuthorSnapshot::query().one(&db).await.unwrap().entries, 1);

		// Inside a transaction, a refresh that rolls back keeps the old rows.
		let tx = db.begin().await.unwrap();
		AuthorSnapshot::refresh(&tx).await.unwrap();
		assert_eq!(AuthorSnapshot::query().one(&tx).await.unwrap().entries, 2);
		tx.rollback().await.unwrap();
		assert_eq!(AuthorSnapshot::query().one(&db).await.unwrap().entries, 1);

		AuthorSnapshot::refresh(&db).await.unwrap();
		assert_eq!(AuthorSnapshot::query().one(&db).await.unwrap().entries, 2);
	})
	.await;
}

#[tokio::test]
async fn views_refuse_every_write() {
	on_both(async |db, _| {
		author(&db, "Ada", &[1]).await;
		let system = db.system("trying to write a view");

		for refused in [
			leptoskit_db::Update::<AuthorTotals>::new()
				.set(|t| t.entries, 3)
				.run(&system)
				.await
				.unwrap_err(),
			AuthorTotals::query().delete(&system).await.unwrap_err(),
			AuthorTotals::query().destroy(&system).await.unwrap_err(),
			leptoskit_db::__private::refresh::<AuthorTotals>(&system, false)
				.await
				.unwrap_err(),
		] {
			assert!(
				matches!(refused, DbError::ReadOnly { ref table, .. } if table == "author_totals"),
				"{refused}"
			);
		}

		const { assert!(!AuthorTotals::MATERIALIZED) };
	})
	.await;
}

#[tokio::test]
async fn migrations_recreate_views_around_table_changes() {
	on_both(async |db, schema| {
		let ada = author(&db, "Ada", &[5]).await;
		AuthorSnapshot::refresh(&db).await.unwrap();

		// Adding a column changes a table the views may read, so they're
		// dropped first and created again last.
		let mut grown = schema.clone();
		let entries = grown
			.tables
			.iter_mut()
			.find(|table| table.name.name == "entries")
			.unwrap();
		entries
			.columns
			.push(Column::new("note", ColumnType::Text { max_len: None }).nullable());
		let plan = leptoskit_db_schema::diff(&schema, &grown, &mut RequireHints).unwrap();
		assert!(
			matches!(plan.steps.first(), Some(Step::DropView(_))),
			"{:?}",
			plan.steps
		);
		assert!(
			matches!(plan.steps.last(), Some(Step::CreateView(_))),
			"{:?}",
			plan.steps
		);
		let migration = leptoskit_db_schema::render(&plan, &schema, &grown);
		db.run_script(&migration.statements.join(";\n"))
			.await
			.unwrap();

		let system = db.system("reading");
		assert_eq!(
			AuthorTotals::query().one(&system).await.unwrap().author_id,
			ada
		);
		// A recreated materialized view is filled as it's created.
		assert_eq!(AuthorSnapshot::query().one(&db).await.unwrap().entries, 1);

		// Changing only a view's query recreates just that view.
		let mut changed = grown.clone();
		let snapshots = changed
			.views
			.iter_mut()
			.find(|view| view.name.name == "author_snapshots")
			.unwrap();
		snapshots.sql = leptoskit_db_schema::Generated::new(
			"SELECT author_id, count(*) * 10 FROM entries GROUP BY author_id",
		);
		let plan = leptoskit_db_schema::diff(&grown, &changed, &mut RequireHints).unwrap();
		assert_eq!(plan.steps.len(), 2, "{:?}", plan.steps);
		let migration = leptoskit_db_schema::render(&plan, &grown, &changed);
		db.run_script(&migration.statements.join(";\n"))
			.await
			.unwrap();
		assert_eq!(AuthorSnapshot::query().one(&db).await.unwrap().entries, 10);

		// Dropping every view leaves the tables alone.
		let mut bare = changed.clone();
		bare.views.clear();
		let plan = leptoskit_db_schema::diff(&changed, &bare, &mut RequireHints).unwrap();
		assert!(
			plan.steps
				.iter()
				.all(|step| matches!(step, Step::DropView(_)))
		);
		assert!(plan.destructive_steps().next().is_none());
		let migration = leptoskit_db_schema::render(&plan, &changed, &bare);
		db.run_script(&migration.statements.join(";\n"))
			.await
			.unwrap();
		assert!(AuthorTotals::query().count(&system).await.is_err());
		assert_eq!(Entry::query().count(&db).await.unwrap(), 1);
	})
	.await;
}

#[tokio::test]
async fn views_read_other_views_and_are_created_after_them() {
	on_both(async |db, schema| {
		// `author_bests` sorts first, but reads `author_totals`.
		let order: Vec<&str> = schema
			.views_in_order()
			.unwrap()
			.iter()
			.map(|view| view.name.name.as_str())
			.collect();
		let position = |name| order.iter().position(|view| *view == name).unwrap();
		assert!(
			position("author_totals") < position("author_bests"),
			"{order:?}"
		);

		let ada = author(&db, "Ada", &[9]).await;
		author(&db, "Grace", &[2]).await;
		let bests = AuthorBest::query().all(&db).await.unwrap();
		assert_eq!(bests.len(), 1);
		assert_eq!((bests[0].author_id, bests[0].best), (ada, 9));

		// Changing the view it reads recreates both, around it.
		let mut changed = schema.clone();
		let totals = changed
			.views
			.iter_mut()
			.find(|view| view.name.name == "author_totals")
			.unwrap();
		totals.sql = leptoskit_db_schema::Generated::new(
			"SELECT author_id, count(*), max(score) * 2 FROM entries GROUP BY author_id",
		);
		let plan = leptoskit_db_schema::diff(&schema, &changed, &mut RequireHints).unwrap();
		let steps: Vec<String> = plan.steps.iter().map(ToString::to_string).collect();
		assert_eq!(
			steps,
			[
				"- view author_bests",
				"- view author_totals",
				"+ view author_totals",
				"+ view author_bests",
			]
		);
		let migration = leptoskit_db_schema::render(&plan, &schema, &changed);
		db.run_script(&migration.statements.join(";\n"))
			.await
			.unwrap();
		assert_eq!(AuthorBest::query().count(&db).await.unwrap(), 1);
		assert_eq!(AuthorBest::query().one(&db).await.unwrap().best, 18);
	})
	.await;
}

#[tokio::test]
async fn indexed_materialized_views_refresh_concurrently() {
	on_both(async |db, schema| {
		let view = schema
			.views
			.iter()
			.find(|view| view.name.name == "author_counts")
			.unwrap();
		assert_eq!(view.indexes.len(), 2);
		assert!(view.indexes.iter().any(|index| index.unique));

		let ada = author(&db, "Ada", &[1, 2]).await;
		AuthorCount::refresh_concurrently(&db).await.unwrap();
		assert_eq!(AuthorCount::query().one(&db).await.unwrap().entries, 2);

		author(&db, "Grace", &[3]).await;
		AuthorCount::refresh_concurrently(&db).await.unwrap();
		assert_eq!(AuthorCount::query().count(&db).await.unwrap(), 2);
		AuthorCount::refresh(&db).await.unwrap();
		assert_eq!(
			AuthorCount::query()
				.filter(|count| count.author_id.eq(ada))
				.one(&db)
				.await
				.unwrap()
				.entries,
			2
		);
	})
	.await;
}

#[tokio::test]
async fn shapes_start_from_views() {
	on_both(async |db, _| {
		let grace = author(&db, "Grace", &[4, 6]).await;
		let cards = AuthorTotals::query()
			.shape::<TotalsCard>()
			.all(&as_author(&db, grace))
			.await
			.unwrap();
		assert_eq!(
			cards,
			[TotalsCard {
				entries: 2,
				author: AuthorName {
					name: "Grace".into()
				},
			}]
		);
		// The view's own policy applies to the shape's rows.
		assert!(
			AuthorTotals::query()
				.shape::<TotalsCard>()
				.all(&db)
				.await
				.unwrap()
				.is_empty()
		);
	})
	.await;
}

/// Who wrote what, and who reads, for the property.
#[derive(Clone, Debug)]
struct Spec {
	/// Each author's scores, and which entries are deleted.
	authors: Vec<Vec<(i64, bool)>>,
	viewer: Option<usize>,
}

fn spec() -> impl Strategy<Value = Spec> {
	(1..4_usize).prop_flat_map(|authors| {
		(
			proptest::collection::vec(
				proptest::collection::vec((0..50_i64, proptest::bool::weighted(0.3)), 0..4),
				authors,
			),
			proptest::option::of(0..authors),
		)
			.prop_map(|(authors, viewer)| Spec { authors, viewer })
	})
}

async fn agrees_with_the_model(db: &Db, spec: Spec) -> Result<(), TestCaseError> {
	let system = db.system("resetting the property");
	Entry::query().destroy(&system).await.unwrap();
	Author::query().destroy(&system).await.unwrap();
	let mut ids = Vec::new();

	for (index, entries) in spec.authors.iter().enumerate() {
		let id = NewAuthor {
			name: format!("a{index}"),
		}
		.insert(&system)
		.await
		.unwrap()
		.id;

		for (score, deleted) in entries {
			let mut entry = NewEntry {
				author_id: id,
				score: *score,
			}
			.insert(&system)
			.await
			.unwrap();

			if *deleted {
				entry.delete(&system).await.unwrap();
			}
		}

		ids.push(id);
	}

	let request = match spec.viewer {
		Some(viewer) => as_author(db, ids[viewer]),
		None => db.with_context(Context::new()),
	};
	let mut seen: Vec<(Id<Author>, i64, i64)> = AuthorTotals::query()
		.all(&request)
		.await
		.unwrap()
		.into_iter()
		.map(|totals| (totals.author_id, totals.entries, totals.best))
		.collect();
	seen.sort();

	let mut expected: Vec<(Id<Author>, i64, i64)> = spec
		.authors
		.iter()
		.enumerate()
		.filter(|(index, _)| spec.viewer == Some(*index))
		.filter_map(|(index, entries)| {
			let live: Vec<i64> = entries
				.iter()
				.filter(|(_, deleted)| !deleted)
				.map(|(score, _)| *score)
				.collect();
			let best = *live.iter().max()?;
			Some((ids[index], live.len() as i64, best))
		})
		.collect();
	expected.sort();
	prop_assert_eq!(seen, expected);
	Ok(())
}

fn run_property(db: &Db, runtime: &tokio::runtime::Runtime, cases: u32) {
	let mut runner = TestRunner::new(Config {
		cases,
		failure_persistence: None,
		..Config::default()
	});
	let outcome = runner.run(&spec(), |spec| {
		runtime.block_on(agrees_with_the_model(db, spec))
	});

	if let Err(error) = outcome {
		panic!("{error}");
	}
}

#[test]
fn view_policies_agree_with_the_model_on_sqlite() {
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let db = runtime.block_on(async {
		let db = Db::connect("sqlite::memory:").await.unwrap();
		create_schema(&db).await;
		db
	});
	run_property(&db, &runtime, 64);
}

#[test]
fn view_policies_agree_with_the_model_on_postgres() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let db = runtime.block_on(async {
		let db = Db::connect(&cluster.database()).await.unwrap();
		create_schema(&db).await;
		db
	});
	run_property(&db, &runtime, 32);
}

/// Unused, but checks a view's policy can deny too.
#[view(name = "hidden_totals", sql = "SELECT author_id FROM entries")]
#[policy(select, |_, _| Expr::always())]
#[policy(deny select, |_, _| Expr::always())]
pub struct HiddenTotals {
	pub author_id: Id<Author>,
}

#[tokio::test]
async fn a_denying_view_policy_hides_every_row() {
	on_both(async |db, _| {
		author(&db, "Ada", &[1]).await;
		assert_eq!(HiddenTotals::query().count(&db).await.unwrap(), 0);
		assert_eq!(
			HiddenTotals::query().count(&db.system("x")).await.unwrap(),
			1
		);
	})
	.await;
}
