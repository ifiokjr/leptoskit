//! Full-text search against both databases: ranking, the query syntax,
//! highlights, every write keeping the index current, raw SQL included, and
//! policies filtering what a search finds.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::SearchWeight;
use leptoskit_db::Searchable;
use leptoskit_db::Segment;
use leptoskit_db::Shape;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest::test_runner::TestCaseError;
use proptest::test_runner::TestRunner;

/// The writer making the request.
struct Viewer(Id<Writer>);

/// Someone who writes articles.
#[table(name = "writers")]
pub struct Writer {
	pub id: Id<Self>,
	pub name: String,
}

/// An article: anyone reads a published one, and its writer reads their
/// drafts.
#[table(name = "articles", search(language = "english"))]
#[policy(select, |article, cx| article.published.eq(true) | cx.global(|viewer: &Viewer| article.writer_id.eq(viewer.0)))]
#[policy(insert, update, delete, |_, _| Expr::always())]
pub struct Article {
	pub id: Id<Self>,
	pub writer_id: Id<Writer>,
	#[db(search(weight = A))]
	pub title: String,
	#[db(search(weight = C))]
	pub body: Option<String>,
	pub published: bool,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// An article as a list of results shows it: its title and its writer.
#[derive(Shape, Debug, PartialEq)]
#[db(from = Article)]
pub struct ArticleCard {
	pub id: Id<Article>,
	pub title: String,
	#[db(via = writer)]
	pub writer: WriterName,
}

#[derive(Shape, Debug, PartialEq)]
#[db(from = Writer)]
pub struct WriterName {
	pub name: String,
}

/// A note in a namespace, keyed by its place, searched in the default
/// language: every word as written, lowercased.
#[table(name = "notes", namespace = "library")]
pub struct Note {
	#[db(key)]
	pub shelf: i32,
	#[db(key)]
	pub slot: i32,
	#[db(search)]
	pub text: String,
}

async fn create_schema(db: &Db) {
	let dialect = db.dialect();
	let schema = leptoskit_db::schema(dialect).unwrap();
	let empty = Schema::empty(dialect);
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
	let migration = leptoskit_db_schema::render(&plan, &empty, &schema);
	db.run_script(&migration.sql()).await.unwrap();
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

async fn writer(db: &Db, name: &str) -> Id<Writer> {
	NewWriter { name: name.into() }.insert(db).await.unwrap().id
}

async fn article(db: &Db, writer_id: Id<Writer>, title: &str, body: Option<&str>) -> Article {
	NewArticle {
		writer_id,
		title: title.into(),
		body: body.map(Into::into),
		published: true,
	}
	.insert(db)
	.await
	.unwrap()
}

fn titles(articles: &[Article]) -> Vec<&str> {
	articles
		.iter()
		.map(|article| article.title.as_str())
		.collect()
}

async fn found(db: &Db, text: &str) -> Vec<String> {
	let mut titles: Vec<String> = Article::query()
		.search(text)
		.all(db)
		.await
		.unwrap()
		.into_iter()
		.map(|article| article.title)
		.collect();
	titles.sort();
	titles
}

#[test]
fn the_macro_declares_what_is_searched() {
	assert_eq!(Article::LANGUAGE, "english");
	assert_eq!(
		Article::SEARCHED,
		&[("title", SearchWeight::A), ("body", SearchWeight::C)]
	);
	assert_eq!(Note::LANGUAGE, "simple");
	assert_eq!(Note::SEARCHED, &[("text", SearchWeight::D)]);

	let search = leptoskit_db::schema(leptoskit_db::Dialect::Postgres)
		.unwrap()
		.tables
		.into_iter()
		.find(|table| table.name.name == "articles")
		.unwrap()
		.search
		.unwrap();
	assert_eq!(search.language, "english");
	assert_eq!(search.columns.len(), 2);
}

#[tokio::test]
async fn matches_in_titles_rank_above_matches_in_bodies() {
	on_both(async |db| {
		let ada = writer(&db, "Ada").await;
		article(&db, ada, "Gardening", Some("Notes on passkeys, briefly")).await;
		article(&db, ada, "Passkeys in Rust", Some("How to sign in")).await;
		article(&db, ada, "Cooking", Some("Nothing to see")).await;

		// English stems both `passkey` and `passkeys` to the same word.
		let hits = Article::query().search("passkey").hits(&db).await.unwrap();
		assert_eq!(
			hits.iter()
				.map(|hit| hit.row.title.as_str())
				.collect::<Vec<_>>(),
			["Passkeys in Rust", "Gardening"]
		);
		assert!(
			hits[0].rank > hits[1].rank,
			"{} > {}",
			hits[0].rank,
			hits[1].rank
		);
		assert!(hits[1].rank > 0.0);

		let first = Article::query()
			.search("passkeys")
			.first(&db)
			.await
			.unwrap();
		assert_eq!(first.unwrap().title, "Passkeys in Rust");
		assert_eq!(
			Article::query()
				.search("passkeys")
				.count(&db)
				.await
				.unwrap(),
			2
		);
		assert_eq!(
			titles(
				&Article::query()
					.search("passkeys")
					.offset(1)
					.all(&db)
					.await
					.unwrap()
			),
			["Gardening"]
		);
	})
	.await;
}

#[tokio::test]
async fn the_query_reads_like_a_search_box() {
	on_both(async |db| {
		let ada = writer(&db, "Ada").await;
		article(&db, ada, "Rust and Go", Some("systems languages")).await;
		article(&db, ada, "Rust alone", Some("memory safety")).await;
		article(&db, ada, "Go alone", Some("simple concurrency")).await;
		article(
			&db,
			ada,
			"Language design",
			Some("safety first, rust later"),
		)
		.await;

		assert_eq!(found(&db, "rust go").await, ["Rust and Go"]);
		assert_eq!(
			found(&db, "rust -go").await,
			["Language design", "Rust alone"]
		);
		assert_eq!(
			found(&db, "concurrency OR memory").await,
			["Go alone", "Rust alone"]
		);
		assert_eq!(found(&db, "\"memory safety\"").await, ["Rust alone"]);
		assert_eq!(found(&db, "\"safety memory\"").await, Vec::<String>::new());
		assert_eq!(found(&db, "concur*").await, ["Go alone"]);
		assert_eq!(
			found(&db, "lang*").await,
			["Language design", "Rust and Go"]
		);
		// Punctuation, case, and stray syntax never make a query invalid.
		assert_eq!(found(&db, "RUST!!! (go) \"").await, ["Rust and Go"]);
		assert_eq!(found(&db, "' OR 1=1 --").await, Vec::<String>::new());
		// Nothing to match finds nothing.
		assert_eq!(found(&db, "").await, [] as [String; 0]);
		assert_eq!(found(&db, "-rust").await, [] as [String; 0]);
		assert_eq!(Article::query().search("??").count(&db).await.unwrap(), 0);
		// Filters and orderings apply too: ties break by the ordering.
		let alone = Article::query()
			.search("alone")
			.filter(|article| article.title.ne("Go alone"))
			.all(&db)
			.await
			.unwrap();
		assert_eq!(titles(&alone), ["Rust alone"]);
		let ordered = Article::query()
			.search("alone")
			.order_by(|article| article.title.desc())
			.all(&db)
			.await
			.unwrap();
		assert_eq!(titles(&ordered), ["Rust alone", "Go alone"]);
	})
	.await;
}

#[tokio::test]
async fn highlights_mark_the_matching_words() {
	on_both(async |db| {
		let ada = writer(&db, "Ada").await;
		let body = "Passkeys replace passwords. Every account gets passkeys, and nobody types a \
		            password again, which is safer for everyone involved in signing in.";
		article(&db, ada, "Passkeys <b>now</b>", Some(body)).await;

		let hits = Article::query()
			.search("passkey")
			.highlight(|article| article.title)
			.excerpt(|article| article.body, 8)
			.hits(&db)
			.await
			.unwrap();
		let [hit] = hits.as_slice() else {
			panic!("one hit, not {}", hits.len());
		};
		let [title, excerpt] = hit.highlights.as_slice() else {
			panic!("two highlights");
		};

		// Text, never HTML: markup in the row stays as written.
		assert_eq!(
			title.segments(),
			&[
				Segment {
					text: "Passkeys".into(),
					matched: true
				},
				Segment {
					text: " <b>now</b>".into(),
					matched: false
				},
			]
		);
		assert_eq!(title.text(), "Passkeys <b>now</b>");
		assert!(
			excerpt
				.matches()
				.all(|word| word == "Passkeys" || word == "passkeys"),
			"{excerpt:?}"
		);
		assert!(excerpt.matches().count() >= 1, "{excerpt:?}");
		let words = excerpt.text().split_whitespace().count();
		assert!((1..=10).contains(&words), "{words} words: {excerpt:?}");
		assert!(body.contains(excerpt.text().trim()), "{excerpt:?}");

		// A row without the column's text highlights nothing.
		article(&db, ada, "Passkeys tomorrow", None).await;
		let hits = Article::query()
			.search("tomorrow passkeys")
			.highlight(|article| article.body)
			.hits(&db)
			.await
			.unwrap();
		assert_eq!(hits.len(), 1);
		assert_eq!(hits[0].highlights[0].segments(), [] as [Segment; 0]);
	})
	.await;
}

#[tokio::test]
async fn private_use_markers_in_text_never_split_highlights() {
	on_both(async |db| {
		let ada = writer(&db, "Ada").await;
		article(&db, ada, "odd \u{E000}marker\u{E001} title", None).await;
		let hits = Article::query()
			.search("title")
			.highlight(|article| article.title)
			.hits(&db)
			.await
			.unwrap();
		assert_eq!(
			hits[0].highlights[0].matches().collect::<Vec<_>>(),
			["title"]
		);
		assert_eq!(hits[0].highlights[0].text(), "odd marker title");
		// The row itself keeps them.
		assert!(hits[0].row.title.contains('\u{E000}'));
	})
	.await;
}

#[tokio::test]
async fn highlights_must_name_searched_columns() {
	on_both(async |db| {
		let missing = Article::query()
			.search("x")
			.highlight(|article| article.title)
			.highlight(|_| leptoskit_db::Col::<Article, String>::new("published"))
			.hits(&db)
			.await;
		assert!(
			matches!(&missing, Err(DbError::Encode(error)) if error.to_string().contains("isn't searched")),
			"{missing:?}"
		);

		for words in [0, 1, 65] {
			let wrong = Article::query()
				.search("x")
				.excerpt(|article| article.title, words)
				.count(&db)
				.await;
			assert!(
				matches!(wrong, Err(DbError::Encode(_))),
				"{words}: {wrong:?}"
			);
		}

		let long = vec!["word"; 33].join(" ");
		assert!(matches!(
			Article::query().search(&long).all(&db).await,
			Err(DbError::Encode(_))
		));
	})
	.await;
}

#[tokio::test]
async fn every_write_keeps_the_index_current() {
	on_both(async |db| {
		let ada = writer(&db, "Ada").await;
		let mut first = article(&db, ada, "Alpha", Some("bravo")).await;
		let second = article(&db, ada, "Charlie", None).await;
		assert_eq!(found(&db, "alpha").await, ["Alpha"]);

		// Saving a row.
		first.title = "Delta".into();
		first.save(&db).await.unwrap();
		assert_eq!(found(&db, "alpha").await, [] as [String; 0]);
		assert_eq!(found(&db, "delta bravo").await, ["Delta"]);

		// Bulk updates.
		Article::update()
			.filter(|article| article.id.eq(second.id))
			.set(|article| article.body, Some("echo".to_string()))
			.run(&db)
			.await
			.unwrap();
		assert_eq!(found(&db, "echo").await, ["Charlie"]);

		let golf = article(&db, ada, "Golf", Some("hotel")).await;
		assert_eq!(found(&db, "hotel").await, ["Golf"]);

		// Soft deletes hide a row; destroying it removes it from the index.
		let mut deleted = Article::get(&db, golf.id).await.unwrap();
		deleted.delete(&db).await.unwrap();
		assert_eq!(found(&db, "golf").await, [] as [String; 0]);
		assert_eq!(
			titles(
				&Article::query()
					.search("golf")
					.with_deleted()
					.all(&db)
					.await
					.unwrap()
			),
			["Golf"]
		);
		Article::query()
			.filter(|article| article.id.eq(golf.id))
			.destroy(&db)
			.await
			.unwrap();
		assert!(
			Article::query()
				.search("golf")
				.with_deleted()
				.all(&db)
				.await
				.unwrap()
				.is_empty()
		);

		// Writes the database layer never sees: raw SQL.
		db.run_script("UPDATE articles SET title = 'India' WHERE title = 'Charlie'")
			.await
			.unwrap();
		assert_eq!(found(&db, "india").await, ["India"]);
		assert_eq!(found(&db, "charlie").await, [] as [String; 0]);

		// Bulk destroys.
		Article::query().destroy(&db).await.unwrap();
		assert_eq!(found(&db, "india").await, [] as [String; 0]);
		assert_eq!(found(&db, "delta").await, [] as [String; 0]);
	})
	.await;
}

#[tokio::test]
async fn composite_keys_and_namespaces_are_searched_too() {
	on_both(async |db| {
		for (shelf, slot, text) in [
			(1, 1, "red apples"),
			(1, 2, "green apples"),
			(2, 1, "pears"),
		] {
			NewNote {
				shelf,
				slot,
				text: text.into(),
			}
			.insert(&db)
			.await
			.unwrap();
		}

		let apples = Note::query()
			.search("apples")
			.order_by(|note| note.slot.desc())
			.all(&db)
			.await
			.unwrap();
		let places: Vec<(i32, i32)> = apples.iter().map(|note| (note.shelf, note.slot)).collect();
		// Equal ranks, so the ordering decides.
		assert_eq!(places, [(1, 2), (1, 1)]);

		// The simple language doesn't stem.
		assert!(
			Note::query()
				.search("apple")
				.all(&db)
				.await
				.unwrap()
				.is_empty()
		);

		let mut note = Note::get_by_place(&db, 2, 1).await;
		note.text = "plums".into();
		note.save(&db).await.unwrap();
		assert_eq!(Note::query().search("plums").count(&db).await.unwrap(), 1);
		assert_eq!(Note::query().search("pears").count(&db).await.unwrap(), 0);

		// An upsert that finds the row updates its words.
		NewNote {
			shelf: 2,
			slot: 1,
			text: "quinces".into(),
		}
		.upsert(&db, |note| (note.shelf, note.slot))
		.await
		.unwrap();
		assert_eq!(Note::query().search("quinces").count(&db).await.unwrap(), 1);
		assert_eq!(Note::query().search("plums").count(&db).await.unwrap(), 0);

		// Moving a row to another key moves its words with it.
		Note::update()
			.filter(|note| note.shelf.eq(2))
			.set(|note| note.shelf, 3)
			.run(&db)
			.await
			.unwrap();
		let moved = Note::query()
			.search("quinces")
			.first(&db)
			.await
			.unwrap()
			.unwrap();
		assert_eq!((moved.shelf, moved.slot), (3, 1));
	})
	.await;
}

impl Note {
	async fn get_by_place(db: &Db, shelf: i32, slot: i32) -> Self {
		Self::query()
			.filter(|note| note.shelf.eq(shelf) & note.slot.eq(slot))
			.one(db)
			.await
			.unwrap()
	}
}

#[tokio::test]
async fn searches_find_only_what_the_request_may_read() {
	on_both(async |db| {
		let ada = writer(&db, "Ada").await;
		let grace = writer(&db, "Grace").await;
		let system = db.system("writing the fixtures");
		article(&system, ada, "Published rust", None).await;
		let mut draft = article(&system, ada, "Draft rust", None).await;
		draft.published = false;
		draft.save(&system).await.unwrap();

		let as_ada = db.with_context(Context::new().with(Viewer(ada)));
		let as_grace = db.with_context(Context::new().with(Viewer(grace)));
		let signed_out = db.with_context(Context::new());

		assert_eq!(
			found(&as_ada, "rust").await,
			["Draft rust", "Published rust"]
		);
		assert_eq!(found(&as_grace, "rust").await, ["Published rust"]);
		assert_eq!(found(&signed_out, "rust").await, ["Published rust"]);
		assert_eq!(
			found(&system, "rust").await,
			["Draft rust", "Published rust"]
		);
		assert_eq!(
			Article::query()
				.search("draft")
				.count(&as_grace)
				.await
				.unwrap(),
			0
		);
		assert!(
			Article::query()
				.search("draft")
				.highlight(|article| article.title)
				.hits(&as_grace)
				.await
				.unwrap()
				.is_empty()
		);
	})
	.await;
}

#[tokio::test]
async fn searches_return_shapes_ranked_and_highlighted() {
	on_both(async |db| {
		let ada = writer(&db, "Ada").await;
		let grace = writer(&db, "Grace").await;
		let system = db.system("writing the fixtures");
		article(&system, ada, "Passkeys in rust", Some("A guide.")).await;
		article(&system, grace, "Notes", Some("Mostly about rust.")).await;
		let mut draft = article(&system, grace, "Draft rust", None).await;
		draft.published = false;
		draft.save(&system).await.unwrap();
		let as_ada = db.with_context(Context::new().with(Viewer(ada)));

		// Title matches rank above body matches, nested rows come along, and
		// Grace's draft stays hers.
		let hits = Article::query()
			.search("rust")
			.highlight(|article| article.title)
			.shape::<ArticleCard>()
			.hits(&as_ada)
			.await
			.unwrap();
		let cards: Vec<(&str, &str)> = hits
			.iter()
			.map(|hit| (hit.row.title.as_str(), hit.row.writer.name.as_str()))
			.collect();
		assert_eq!(cards, [("Passkeys in rust", "Ada"), ("Notes", "Grace")]);
		assert!(hits[0].rank > hits[1].rank);
		assert_eq!(
			hits[0].highlights[0].matches().collect::<Vec<_>>(),
			["rust"]
		);
		assert!(hits[1].highlights[0].matches().next().is_none());

		// The search's filter, limit, and orderings carry over.
		let notes = Article::query()
			.search("rust")
			.filter(|article| article.writer_id.eq(grace))
			.shape::<ArticleCard>()
			.first(&as_ada)
			.await
			.unwrap();
		assert_eq!(notes.map(|card| card.title), Some("Notes".to_owned()));
		assert_eq!(
			Article::query()
				.search("rust")
				.limit(1)
				.shape::<ArticleCard>()
				.all(&system)
				.await
				.unwrap()
				.len(),
			1
		);
		assert_eq!(
			Article::query()
				.search("nothing")
				.shape::<ArticleCard>()
				.all(&as_ada)
				.await
				.unwrap(),
			Vec::<ArticleCard>::new()
		);
	})
	.await;
}

/// The articles a property writes: each with a writer, whether it's
/// published or deleted, and its title and body words.
#[derive(Clone, Debug)]
struct Spec {
	articles: Vec<Written>,
	viewer: Option<usize>,
	query: Vec<(usize, bool)>,
	or: Option<usize>,
}

#[derive(Clone, Debug)]
struct Written {
	writer: usize,
	published: bool,
	deleted: bool,
	title: Vec<usize>,
	body: Option<Vec<usize>>,
}

/// Words that stem to themselves and aren't stopwords in either database.
const WORDS: &[&str] = &["alpha", "bravo", "delta", "kilo", "lima", "oscar"];
const WRITERS: usize = 3;

fn words() -> impl Strategy<Value = Vec<usize>> {
	proptest::collection::vec(0..WORDS.len(), 1..4)
}

fn spec() -> impl Strategy<Value = Spec> {
	(
		proptest::collection::vec(
			(
				0..WRITERS,
				proptest::bool::weighted(0.6),
				proptest::bool::weighted(0.2),
				words(),
				proptest::option::of(words()),
			)
				.prop_map(|(writer, published, deleted, title, body)| {
					Written {
						writer,
						published,
						deleted,
						title,
						body,
					}
				}),
			0..8,
		),
		proptest::option::of(0..WRITERS),
		proptest::collection::vec((0..WORDS.len(), proptest::bool::weighted(0.25)), 1..3),
		proptest::option::of(0..WORDS.len()),
	)
		.prop_map(|(articles, viewer, query, or)| {
			Spec {
				articles,
				viewer,
				query,
				or,
			}
		})
}

impl Spec {
	/// The search box text: each word, `-` for an excluded one, and maybe
	/// `OR` another word.
	fn text(&self) -> String {
		let mut parts: Vec<String> = self
			.query
			.iter()
			.map(|(word, excluded)| format!("{}{}", if *excluded { "-" } else { "" }, WORDS[*word]))
			.collect();

		if let Some(word) = self.or {
			parts.push("OR".into());
			parts.push(WORDS[word].into());
		}

		parts.join(" ")
	}

	/// Whether the model says `article` matches and the viewer reads it.
	fn finds(&self, article: &Written) -> bool {
		let has = |word: usize| {
			article.title.contains(&word)
				|| article
					.body
					.as_ref()
					.is_some_and(|body| body.contains(&word))
		};
		let first = self.query.iter().any(|(_, excluded)| !excluded)
			&& self
				.query
				.iter()
				.all(|(word, excluded)| has(*word) != *excluded);
		let second = self.or.is_some_and(has);
		let visible = article.published || self.viewer == Some(article.writer);

		(first || second) && visible && !article.deleted
	}
}

fn sentence(words: &[usize]) -> String {
	words
		.iter()
		.map(|word| WORDS[*word])
		.collect::<Vec<_>>()
		.join(" ")
}

async fn agrees_with_the_model(db: &Db, spec: Spec) -> Result<(), TestCaseError> {
	let system = db.system("resetting the property");
	Article::query().destroy(&system).await.unwrap();
	Writer::query().destroy(&system).await.unwrap();
	let mut writers = Vec::new();

	for index in 0..WRITERS {
		writers.push(writer(&system, &format!("w{index}")).await);
	}

	let mut expected = Vec::new();

	for written in &spec.articles {
		let mut row = NewArticle {
			writer_id: writers[written.writer],
			title: sentence(&written.title),
			body: written.body.as_deref().map(sentence),
			published: written.published,
		}
		.insert(&system)
		.await
		.unwrap();

		if written.deleted {
			row.delete(&system).await.unwrap();
		}

		if spec.finds(written) {
			expected.push(row.id);
		}
	}

	let request = match spec.viewer {
		Some(viewer) => db.with_context(Context::new().with(Viewer(writers[viewer]))),
		None => db.with_context(Context::new()),
	};
	let hits = Article::query()
		.search(&spec.text())
		.hits(&request)
		.await
		.unwrap();

	for pair in hits.windows(2) {
		prop_assert!(pair[0].rank >= pair[1].rank, "ranks descend");
	}

	let mut seen: Vec<Id<Article>> = hits.into_iter().map(|hit| hit.row.id).collect();
	seen.sort();
	expected.sort();
	prop_assert_eq!(&seen, &expected, "searching {:?}", spec.text());
	let count = Article::query()
		.search(&spec.text())
		.count(&request)
		.await
		.unwrap();
	prop_assert_eq!(count, expected.len() as u64);
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
fn searches_agree_with_the_model_on_sqlite() {
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let db = runtime.block_on(async {
		let db = Db::connect("sqlite::memory:").await.unwrap();
		create_schema(&db).await;
		db
	});
	run_property(&db, &runtime, 96);
}

#[test]
fn searches_agree_with_the_model_on_postgres() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let db = runtime.block_on(async {
		let db = Db::connect(&cluster.database()).await.unwrap();
		create_schema(&db).await;
		db
	});
	run_property(&db, &runtime, 48);
}
