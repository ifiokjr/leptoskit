//! Filters that reach through links, distinct reads, regex matching, and
//! totals, against both databases.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use std::future::Future;

use leptoskit_db::Avg;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Max;
use leptoskit_db::Min;
use leptoskit_db::Sum;
use leptoskit_db::table;
use leptoskit_db::view;
use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;

/// Someone who writes.
#[table(name = "writers")]
pub struct Writer {
	pub id: Id<Self>,
	pub handle: String,
	pub karma: i32,
}

/// What they wrote, and who edited it.
#[table(name = "stories")]
pub struct Story {
	pub id: Id<Self>,
	#[db(backlink = stories)]
	pub author_id: Id<Writer>,
	pub editor_id: Option<Id<Writer>>,
	pub title: String,
}

/// Every story's title, with repeats.
#[view(name = "story_titles", sql = "SELECT title FROM stories")]
pub struct StoryTitle {
	pub title: String,
}

async fn create_schema(db: &Db) {
	let dialect = db.dialect();
	let schema = leptoskit_db::schema(dialect).unwrap();
	let empty = Schema::empty(dialect);
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
	let migration = leptoskit_db_schema::render(&plan, &empty, &schema);
	db.run_script(&migration.statements.join(";\n"))
		.await
		.unwrap();
}

async fn on_both<F, Fut>(check: F)
where
	F: Fn(Db) -> Fut,
	Fut: Future<Output = ()>,
{
	let db = Db::connect("sqlite::memory:").await.unwrap();
	create_schema(&db).await;
	check(db).await;

	if let Some(cluster) = support::postgres::PostgresCluster::start() {
		let db = Db::connect(&cluster.database()).await.unwrap();
		create_schema(&db).await;
		check(db).await;
	}
}

/// Ada with karma 10 and Grace with karma 20, each with a story, and one
/// story Ada edited.
async fn fixture(db: &Db) -> (Writer, Writer) {
	let ada = NewWriter {
		handle: "ada".into(),
		karma: 10,
	}
	.insert(db)
	.await
	.unwrap();
	let grace = NewWriter {
		handle: "grace".into(),
		karma: 20,
	}
	.insert(db)
	.await
	.unwrap();
	for (author, title) in [(&ada, "Engines"), (&grace, "Compilers")] {
		NewStory {
			author_id: author.id,
			editor_id: None,
			title: title.into(),
		}
		.insert(db)
		.await
		.unwrap();
	}
	NewStory {
		author_id: grace.id,
		editor_id: Some(ada.id),
		title: "Notes".into(),
	}
	.insert(db)
	.await
	.unwrap();
	(ada, grace)
}

#[tokio::test(flavor = "multi_thread")]
async fn link_filters_follow_the_row_they_point_at() {
	on_both(async |db| {
		let _ = fixture(&db).await;

		let adas = Story::query()
			.filter(|story| story.author_id.has(|writer| writer.handle.eq("ada")))
			.all(&db)
			.await
			.unwrap();
		assert_eq!(adas.len(), 1);
		assert_eq!(adas[0].title, "Engines");

		// `NOT EXISTS`, through the condition combinators.
		let others = Story::query()
			.filter(|story| !story.author_id.has(|writer| writer.handle.eq("ada")))
			.count(&db)
			.await
			.unwrap();
		assert_eq!(others, 2);

		// An optional link's `has` matches nothing while it is `NULL`, and
		// the row it names once it points.
		let edited = Story::query()
			.filter(|story| story.editor_id.has(|writer| writer.karma.gte(10)))
			.count(&db)
			.await
			.unwrap();
		assert_eq!(edited, 1, "only the story Ada edited");
		let unedited = Story::query()
			.filter(|story| story.editor_id.is_null())
			.count(&db)
			.await
			.unwrap();
		assert_eq!(unedited, 2);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn distinct_reads_each_row_once() {
	on_both(async |db| {
		let (_, grace) = fixture(&db).await;
		NewStory {
			author_id: grace.id,
			editor_id: None,
			title: "Notes".into(),
		}
		.insert(&db)
		.await
		.unwrap();

		// A view's rows repeat; `distinct` reads each once. Whole tables
		// never repeat a row, since their keys are unique.
		assert_eq!(StoryTitle::query().count(&db).await.unwrap(), 4);
		assert_eq!(StoryTitle::query().distinct().count(&db).await.unwrap(), 3);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn totals_read_matching_rows() {
	on_both(async |db| {
		fixture(&db).await;

		let everyone = Writer::query();
		assert_eq!(
			everyone
				.clone()
				.aggregate(|writer| Sum(writer.karma), &db)
				.await
				.unwrap(),
			Some(30)
		);
		assert_eq!(
			everyone
				.clone()
				.aggregate(|writer| Avg(writer.karma), &db)
				.await
				.unwrap(),
			Some(15.0)
		);
		assert_eq!(
			everyone
				.clone()
				.aggregate(|writer| Min(writer.handle), &db)
				.await
				.unwrap(),
			Some("ada".to_owned())
		);
		assert_eq!(
			everyone
				.aggregate(|writer| Max(writer.handle), &db)
				.await
				.unwrap(),
			Some("grace".to_owned())
		);

		// No row matching makes every total `None`, as SQL does.
		let none = Writer::query().filter(|writer| writer.karma.gt(100));
		assert_eq!(
			none.aggregate(|writer| Sum(writer.karma), &db)
				.await
				.unwrap(),
			None
		);
		assert_eq!(
			Writer::query()
				.filter(|writer| writer.karma.gt(100))
				.aggregate(|writer| Max(writer.karma), &db)
				.await
				.unwrap(),
			None
		);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn regex_filters_match_text_on_postgres_only() {
	on_both(async |db| {
		fixture(&db).await;

		let query = || Writer::query().filter(|writer| writer.handle.matches_regex("^g"));

		match db.dialect() {
			Dialect::Postgres => {
				let matched = query().all(&db).await.unwrap();
				assert_eq!(matched.len(), 1);
				assert_eq!(matched[0].handle, "grace");
			}
			Dialect::Sqlite => {
				let error = query().all(&db).await.unwrap_err();
				assert!(format!("{error:?}").contains("Postgres"), "{error:?}");
			}
		}
	})
	.await;
}
