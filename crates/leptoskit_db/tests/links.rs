//! Following links both ways: accessors for the row an `Id<T>` column points
//! at, `by_…` queries, and backlinks, against both databases.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use leptoskit_db::Connection;
use leptoskit_db::Db;
use leptoskit_db::HasId;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;

/// Someone who writes.
#[table(name = "writers")]
pub struct Writer {
	pub id: Id<Self>,
	pub name: String,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// Something a writer wrote, and maybe an editor checked.
#[table(name = "articles")]
pub struct Article {
	pub id: Id<Self>,
	#[db(backlink = articles)]
	pub author_id: Id<Writer>,
	#[db(backlink = edited)]
	pub editor_id: Option<Id<Writer>>,
	pub title: String,
}

/// A comment, possibly replying to another.
#[table(name = "remarks")]
pub struct Remark {
	pub id: Id<Self>,
	pub article_id: Id<Article>,
	#[db(backlink = replies)]
	pub parent_id: Option<Id<Remark>>,
	pub body: String,
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

async fn writer(db: &Db, name: &str) -> Writer {
	NewWriter { name: name.into() }.insert(db).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn links_load_the_rows_they_point_at() {
	on_both(async |db| {
		let ada = writer(&db, "Ada").await;
		let grace = writer(&db, "Grace").await;
		let article = NewArticle {
			author_id: ada.id,
			editor_id: None,
			title: "Engines".into(),
		}
		.insert(&db)
		.await
		.unwrap();

		assert_eq!(article.author(&db).await.unwrap().name, "Ada");
		assert!(article.editor(&db).await.unwrap().is_none());

		let mut edited = article;
		edited.editor_id = Some(grace.id);
		edited.save(&db).await.unwrap();
		assert_eq!(edited.editor(&db).await.unwrap().unwrap().id(), grace.id);

		// Soft-deleted rows are gone to links, as to every other query.
		let mut gone = Writer::get(&db, grace.id).await.unwrap();
		gone.delete(&db).await.unwrap();
		assert!(edited.editor(&db).await.unwrap().is_none());
		let mut ada = ada;
		ada.delete(&db).await.unwrap();
		assert!(edited.author(&db).await.unwrap_err().is_not_found());
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn backlinks_list_the_rows_pointing_back() {
	on_both(async |db| {
		let ada = writer(&db, "Ada").await;
		let grace = writer(&db, "Grace").await;

		for (title, editor) in [("A", None), ("B", Some(grace.id)), ("C", Some(grace.id))] {
			NewArticle {
				author_id: ada.id,
				editor_id: editor,
				title: title.into(),
			}
			.insert(&db)
			.await
			.unwrap();
		}

		let titles: Vec<String> = ada
			.articles()
			.order_by(|article| article.title.desc())
			.all(&db)
			.await
			.unwrap()
			.into_iter()
			.map(|article| article.title)
			.collect();
		assert_eq!(titles, ["C", "B", "A"]);
		assert_eq!(grace.articles().count(&db).await.unwrap(), 0);
		assert_eq!(grace.edited().count(&db).await.unwrap(), 2);
		assert_eq!(
			Article::by_editor(grace.id)
				.filter(|article| article.title.eq("B"))
				.count(&db)
				.await
				.unwrap(),
			1
		);

		// A table can link to itself.
		let article = Article::by_author(ada.id)
			.first(&db)
			.await
			.unwrap()
			.unwrap();
		let question = NewRemark {
			article_id: article.id,
			parent_id: None,
			body: "Why?".into(),
		}
		.insert(&db)
		.await
		.unwrap();
		let answer = NewRemark {
			article_id: article.id,
			parent_id: Some(question.id),
			body: "Because.".into(),
		}
		.insert(&db)
		.await
		.unwrap();
		assert_eq!(answer.parent(&db).await.unwrap().unwrap().body, "Why?");
		assert_eq!(answer.article(&db).await.unwrap().id, article.id);
		let replies = question.replies().all(&db).await.unwrap();
		assert_eq!(replies.len(), 1);
		assert_eq!(replies[0].id, answer.id);
	})
	.await;
}
