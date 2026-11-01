//! Shapes, `#[derive(db::Shape)]`, against both databases: the SQL they
//! compile to, nested rows through links, backlinks, and link tables, every
//! column type inside a nested row, policies and soft deletes at every
//! level, and a property test comparing nested results with a plain Rust
//! model.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use chrono::NaiveDate;
use leptoskit_db as db;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::Shape;
use leptoskit_db::Slug;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest::test_runner::TestCaseError;
use proptest::test_runner::TestRunner;

/// The signed-in author: the global the policies read.
struct Viewer(Id<Author>);

/// Someone who writes. No policies, but authors can be soft-deleted.
#[table(name = "authors", public)]
pub struct Author {
	pub id: Id<Self>,
	#[db(unique)]
	pub handle: String,
	pub name: String,
	#[db(private)]
	pub email: String,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// What kind of post it is.
#[derive(db::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
	Essay,
	Note,
}

/// Anything a post carries as JSON.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Meta {
	pub words: u32,
	pub draft_of: Option<String>,
}

/// A post: published ones are for everyone, drafts for their author. It
/// holds one column of every kind, so nested rows decode each of them.
#[table(name = "posts", public)]
#[policy(select, |post, cx| post.published.eq(true) | own(post, cx))]
#[policy(insert, update, delete, |post, cx| own(post, cx))]
pub struct Post {
	pub id: Id<Self>,
	#[db(backlink = posts)]
	pub author_id: Id<Author>,
	#[db(backlink = edited)]
	pub editor_id: Option<Id<Author>>,
	pub title: String,
	pub published: bool,
	pub score: f64,
	pub rank: i16,
	pub views: i64,
	#[db(private)]
	pub cover: Option<Vec<u8>>,
	pub meta: Json<Meta>,
	pub day: NaiveDate,
	pub kind: Kind,
	pub slug: Slug,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

fn own(post: &PostColumns, cx: &Context) -> Expr {
	cx.global(|viewer: &Viewer| post.author_id.eq(viewer.0))
}

/// A tag. No policies.
#[table(name = "tags", public)]
pub struct Tag {
	pub id: Id<Self>,
	pub name: String,
}

/// A tag on a post: private ones are for the post's author alone.
#[db::link(name = "post_tags", from = Post as tags, to = Tag as posts)]
#[policy(select, |link, cx| link.private.eq(false) | cx.global(|viewer: &Viewer| cx.exists(Post::query()
	.with_deleted()
	.filter(|post| post.id.eq(link.post_id) & post.author_id.eq(viewer.0)))))]
pub struct PostTag {
	pub private: bool,
}

/// A comment on a post, maybe replying to another.
#[table(name = "comments")]
pub struct Comment {
	pub id: Id<Self>,
	#[db(backlink = comments)]
	pub post_id: Id<Post>,
	#[db(backlink = replies)]
	pub parent_id: Option<Id<Comment>>,
	pub body: String,
}

/// A vote on a post, keyed by the post and the voter together.
#[table(name = "votes")]
pub struct Vote {
	#[db(key, on_delete = cascade, backlink = votes)]
	pub post_id: Id<Post>,
	#[db(key)]
	pub voter: String,
	pub up: bool,
	pub weight: i16,
}

/// An author's posts, soft-deleted ones too, next to the count without.
#[derive(Shape, Debug, PartialEq)]
#[db(from = Author)]
pub struct Archive {
	pub handle: String,
	#[db(via = posts, with_deleted, order_by = |post| post.rank.asc())]
	pub posts: Vec<RankOnly>,
	#[db(count(posts), with_deleted)]
	pub every: i64,
	#[db(count(posts))]
	pub live: i64,
}

#[derive(Shape, Debug, PartialEq)]
#[db(from = Post)]
pub struct RankOnly {
	pub rank: i16,
	#[db(via = author, with_deleted)]
	pub author: HandleOnly,
}

#[derive(Shape, Debug, PartialEq)]
#[db(from = Author)]
pub struct HandleOnly {
	pub handle: String,
}

/// A post's votes: narrowed twice, paged, and ordered by the votes'
/// composite key once the explicit order runs out.
#[derive(Shape, Debug, PartialEq)]
#[db(from = Post)]
pub struct Tally {
	pub title: String,
	#[db(
		via = votes,
		filter = |vote| vote.up.eq(true),
		filter = |vote| vote.weight.gt(0),
		order_by = |vote| vote.weight.desc(),
		offset = 1
	)]
	pub after_the_heaviest: Vec<Voter>,
	#[db(via = votes)]
	pub everyone: Vec<Voter>,
}

#[derive(Shape, Debug, PartialEq)]
#[db(from = Vote)]
pub struct Voter {
	pub voter: String,
}

/// A profile, as a client sees it.
#[derive(Shape, Debug, PartialEq)]
#[db(from = Author, public)]
pub struct ProfileCard {
	pub handle: String,
	#[db(column = name)]
	pub display_name: String,
	#[db(count(posts))]
	pub post_count: i64,
	#[db(count(posts), filter = |post| post.published.eq(false))]
	pub drafts: u64,
	#[db(via = posts, order_by = |post| post.rank.desc(), limit = 2)]
	pub top: Vec<PostTitle>,
	#[db(via = posts, filter = |post| post.title.contains("pinned"), order_by = |post| post.rank.asc())]
	pub pinned: Option<PostTitle>,
}

/// A post's title and tags.
#[derive(Shape, Debug, PartialEq)]
#[db(from = Post, public)]
pub struct PostTitle {
	pub id: Id<Post>,
	pub title: String,
	#[db(via = tags, order_by = |tag| tag.name.asc())]
	pub tags: Vec<TagName>,
	#[db(count(comments))]
	pub comments: i64,
}

#[derive(Shape, Clone, Debug, PartialEq)]
#[db(from = Tag, public)]
pub struct TagName {
	pub name: String,
}

/// A post with who wrote and edited it: server-side, so a private column
/// may be read.
#[derive(Shape, Debug, PartialEq)]
#[db(from = Post)]
pub struct Byline {
	pub title: String,
	#[db(via = author)]
	pub author: Contact,
	#[db(via = editor)]
	pub editor: Option<Contact>,
}

#[derive(Shape, Debug, PartialEq)]
#[db(from = Author)]
pub struct Contact {
	pub name: String,
	pub email: String,
}

/// Every column of a post, nested.
#[derive(Shape, Debug, PartialEq)]
#[db(from = Author)]
pub struct Everything {
	#[db(via = posts)]
	pub posts: Vec<PostEverything>,
}

#[derive(Shape, Debug, PartialEq)]
#[db(from = Post)]
pub struct PostEverything {
	pub id: Id<Post>,
	pub author_id: Id<Author>,
	pub editor_id: Option<Id<Author>>,
	pub title: String,
	pub published: bool,
	pub score: f64,
	pub rank: i16,
	pub views: i64,
	pub cover: Option<Vec<u8>>,
	pub meta: Json<Meta>,
	pub day: NaiveDate,
	pub kind: Kind,
	pub slug: Slug,
	pub created_at: Timestamp,
	pub deleted_at: Option<Timestamp>,
}

/// A comment thread three levels deep: each level follows `replies` from
/// the one above it, through the same table.
#[derive(Shape, Debug, PartialEq)]
#[db(from = Comment)]
pub struct Thread {
	pub body: String,
	#[db(via = replies, order_by = |reply| reply.body.asc())]
	pub replies: Vec<Reply>,
}

#[derive(Shape, Debug, PartialEq)]
#[db(from = Comment)]
pub struct Reply {
	pub body: String,
	#[db(via = replies, order_by = |reply| reply.body.asc())]
	pub replies: Vec<Leaf>,
	#[db(via = parent)]
	pub parent: Option<Leaf>,
}

#[derive(Shape, Debug, PartialEq)]
#[db(from = Comment)]
pub struct Leaf {
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

async fn author(db: &Db, handle: &str) -> Author {
	NewAuthor {
		handle: handle.into(),
		name: format!("{handle}'s name"),
		email: format!("{handle}@example.com"),
	}
	.insert(db)
	.await
	.unwrap()
}

fn as_author(db: &Db, author: &Author) -> Db {
	db.with_context(Context::new().with(Viewer(author.id)))
}

fn new_post(author: &Author, title: &str, published: bool, rank: i16) -> NewPost {
	NewPost {
		author_id: author.id,
		editor_id: None,
		title: title.into(),
		published,
		score: 0.1,
		rank,
		views: 9_007_199_254_740_993,
		cover: Some(vec![0, 1, 254, 255]),
		meta: Json(Meta {
			words: 1200,
			draft_of: Some("ünïcode \"quoted\"".into()),
		}),
		day: NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(),
		kind: Kind::Essay,
		slug: Slug::parse("hello-world").unwrap(),
	}
}

async fn post(db: &Db, author: &Author, title: &str, published: bool, rank: i16) -> Post {
	new_post(author, title, published, rank)
		.insert(&as_author(db, author))
		.await
		.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shape_is_one_statement_on_each_dialect() {
	on_both(async |db| {
		let viewer = author(&db, "ada").await;
		let request = as_author(&db, &viewer);
		let statement = Author::query()
			.filter(|author| author.handle.eq("ada"))
			.order_by(|author| author.handle.asc())
			.limit(5)
			.shape::<ProfileCard>()
			.statement(&request)
			.unwrap();
		insta::assert_snapshot!(format!("profile_card__{}", db.dialect()), statement.sql);

		// A required link, an optional one, and a self-referencing tree.
		let statement = Comment::query()
			.filter(|comment| comment.parent_id.is_null())
			.shape::<Thread>()
			.statement(&db)
			.unwrap();
		insta::assert_snapshot!(format!("thread__{}", db.dialect()), statement.sql);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shapes_fetch_counts_and_nested_rows() {
	on_both(async |db| {
		let ada = author(&db, "ada").await;
		let grace = author(&db, "grace").await;
		let as_ada = as_author(&db, &ada);
		let engines = post(&db, &ada, "Engines", true, 3).await;
		let looms = post(&db, &ada, "Looms (pinned)", true, 1).await;
		let draft = post(&db, &ada, "Draft", false, 9).await;
		post(&db, &grace, "Compilers", true, 5).await;

		let maths = NewTag {
			name: "maths".into(),
		}
		.insert(&db)
		.await
		.unwrap();
		let history = NewTag {
			name: "history".into(),
		}
		.insert(&db)
		.await
		.unwrap();
		// The link table has no rule for adding tags, so only the system may.
		let system = db.system("tagging posts");
		let open = || PostTagProperties { private: false };
		engines.tags().add(&system, maths.id, open()).await.unwrap();
		engines
			.tags()
			.add(&system, history.id, open())
			.await
			.unwrap();
		NewComment {
			post_id: engines.id,
			parent_id: None,
			body: "First".into(),
		}
		.insert(&db)
		.await
		.unwrap();

		let tags = vec![
			TagName {
				name: "history".into(),
			},
			TagName {
				name: "maths".into(),
			},
		];

		// Others see only what's published: the draft isn't counted.
		let card = Author::query()
			.filter(|author| author.handle.eq("ada"))
			.shape::<ProfileCard>()
			.one(&db)
			.await
			.unwrap();
		assert_eq!(
			card,
			ProfileCard {
				handle: "ada".into(),
				display_name: "ada's name".into(),
				post_count: 2,
				drafts: 0,
				top: vec![
					PostTitle {
						id: engines.id,
						title: "Engines".into(),
						tags: tags.clone(),
						comments: 1,
					},
					PostTitle {
						id: looms.id,
						title: "Looms (pinned)".into(),
						tags: Vec::new(),
						comments: 0,
					},
				],
				pinned: Some(PostTitle {
					id: looms.id,
					title: "Looms (pinned)".into(),
					tags: Vec::new(),
					comments: 0,
				}),
			}
		);

		// Ada sees her draft, which outranks the rest.
		let card = Author::query()
			.filter(|author| author.handle.eq("ada"))
			.shape::<ProfileCard>()
			.one(&as_ada)
			.await
			.unwrap();
		assert_eq!((card.post_count, card.drafts), (3, 1));
		let top: Vec<Id<Post>> = card.top.iter().map(|post| post.id).collect();
		assert_eq!(top, [draft.id, engines.id]);

		// Every author, in order, with no rows for one that wrote nothing.
		author(&db, "alan").await;
		let cards = Author::query()
			.order_by(|author| author.handle.asc())
			.shape::<ProfileCard>()
			.all(&db)
			.await
			.unwrap();
		let handles: Vec<(&str, i64)> = cards
			.iter()
			.map(|card| (card.handle.as_str(), card.post_count))
			.collect();
		assert_eq!(handles, [("ada", 2), ("alan", 0), ("grace", 1)]);
		assert!(cards[1].top.is_empty() && cards[1].pinned.is_none());
		assert!(
			Author::query()
				.filter(|author| author.handle.eq("nobody"))
				.shape::<ProfileCard>()
				.first(&db)
				.await
				.unwrap()
				.is_none()
		);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shapes_follow_links_to_the_rows_they_point_at() {
	on_both(async |db| {
		let ada = author(&db, "ada").await;
		let alan = author(&db, "alan").await;
		let as_ada = as_author(&db, &ada);
		let looms = post(&db, &ada, "Looms (pinned)", true, 1).await;
		post(&db, &ada, "Engines", true, 3).await;

		// Links the other way: a required one and an optional one, reading a
		// private column, which only a server-side shape may.
		let mut edited = Post::get(&as_ada, looms.id).await.unwrap();
		edited.editor_id = Some(alan.id);
		edited.save(&as_ada).await.unwrap();
		let bylines = Post::query()
			.order_by(|post| post.rank.asc())
			.shape::<Byline>()
			.all(&db)
			.await
			.unwrap();
		assert_eq!(
			bylines[0],
			Byline {
				title: "Looms (pinned)".into(),
				author: Contact {
					name: "ada's name".into(),
					email: "ada@example.com".into(),
				},
				editor: Some(Contact {
					name: "alan's name".into(),
					email: "alan@example.com".into(),
				}),
			}
		);
		assert_eq!(bylines[1].editor, None);

		// A soft-deleted author is gone to links, as to every query: a
		// required link to one is not found.
		let mut gone = Author::get(&db, alan.id).await.unwrap();
		gone.delete(&db).await.unwrap();
		let byline = Post::query()
			.filter(|post| post.id.eq(looms.id))
			.shape::<Byline>()
			.one(&db)
			.await
			.unwrap();
		assert_eq!(byline.editor, None);
		let mut ada_gone = Author::get(&db, ada.id).await.unwrap();
		ada_gone.delete(&db).await.unwrap();
		let missing = Post::query()
			.filter(|post| post.id.eq(looms.id))
			.shape::<Byline>()
			.one(&db)
			.await
			.unwrap_err();
		assert!(missing.is_not_found(), "{missing:?}");
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_rows_can_include_soft_deleted_ones() {
	on_both(async |db| {
		let ada = author(&db, "ada").await;
		let as_ada = db.with_context(Context::new().with(Viewer(ada.id)));
		post(&db, &ada, "Engines", true, 1).await;
		let mut looms = post(&db, &ada, "Looms", true, 2).await;
		looms.delete(&as_ada).await.unwrap();
		let mut gone = Author::get(&db, ada.id).await.unwrap();
		gone.delete(&db).await.unwrap();

		let archive = Author::query()
			.with_deleted()
			.shape::<Archive>()
			.one(&db)
			.await
			.unwrap();
		let handle = || {
			HandleOnly {
				handle: "ada".into(),
			}
		};
		assert_eq!(
			archive,
			Archive {
				handle: "ada".into(),
				posts: vec![
					RankOnly {
						rank: 1,
						author: handle(),
					},
					RankOnly {
						rank: 2,
						author: handle(),
					},
				],
				every: 2,
				live: 1,
			}
		);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_rows_narrow_page_and_order_by_any_key() {
	on_both(async |db| {
		let ada = author(&db, "ada").await;
		let engines = post(&db, &ada, "Engines", true, 1).await;

		for (voter, up, weight) in [
			("dee", true, 2),
			("bea", true, 5),
			("cy", false, 9),
			("al", true, 2),
			("ed", true, 0),
		] {
			NewVote {
				post_id: engines.id,
				voter: voter.into(),
				up,
				weight,
			}
			.insert(&db)
			.await
			.unwrap();
		}

		let tally = Post::query().shape::<Tally>().one(&db).await.unwrap();
		let names = |voters: &[Voter]| -> Vec<String> {
			voters.iter().map(|voter| voter.voter.clone()).collect()
		};
		// Up, weighted, heaviest first, skipping it; ties by the key.
		assert_eq!(names(&tally.after_the_heaviest), ["al", "dee"]);
		assert_eq!(names(&tally.everyone), ["al", "bea", "cy", "dee", "ed"]);

		// `one` on nothing is not found.
		let none = Post::query()
			.filter(|post| post.title.eq("Nothing"))
			.shape::<Tally>()
			.one(&db)
			.await
			.unwrap_err();
		assert!(none.is_not_found(), "{none:?}");
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_column_type_survives_a_nested_row() {
	on_both(async |db| {
		let ada = author(&db, "ada").await;
		let grace = author(&db, "grace").await;
		let mut plain = new_post(&ada, "Plain", true, -3);
		plain.editor_id = Some(grace.id);
		plain.cover = None;
		plain.score = -1.5e300;
		plain.views = i64::MIN;
		plain.kind = Kind::Note;
		plain.meta = Json(Meta {
			words: 0,
			draft_of: None,
		});
		let request = as_author(&db, &ada);
		let first = post(&db, &ada, "Full", true, 7).await;
		let second = plain.insert(&request).await.unwrap();

		let everything = Author::query()
			.filter(|author| author.id.eq(ada.id))
			.shape::<Everything>()
			.one(&request)
			.await
			.unwrap();
		let expected: Vec<PostEverything> = [first, second]
			.into_iter()
			.map(|post| {
				PostEverything {
					id: post.id,
					author_id: post.author_id,
					editor_id: post.editor_id,
					title: post.title,
					published: post.published,
					score: post.score,
					rank: post.rank,
					views: post.views,
					cover: post.cover,
					meta: post.meta,
					day: post.day,
					kind: post.kind,
					slug: post.slug,
					created_at: post.created_at,
					deleted_at: post.deleted_at,
				}
			})
			.collect();
		assert_eq!(everything.posts, expected);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_rows_correlate_with_their_own_parent_at_every_depth() {
	on_both(async |db| {
		let ada = author(&db, "ada").await;
		let engines = post(&db, &ada, "Engines", true, 1).await;
		let comment = async |body: &str, parent: Option<Id<Comment>>| {
			NewComment {
				post_id: engines.id,
				parent_id: parent,
				body: body.into(),
			}
			.insert(&db)
			.await
			.unwrap()
			.id
		};
		let root = comment("root", None).await;
		let other = comment("other root", None).await;
		let b = comment("b", Some(root)).await;
		let a = comment("a", Some(root)).await;
		comment("b.1", Some(b)).await;
		comment("a.2", Some(a)).await;
		comment("a.1", Some(a)).await;
		comment("other.1", Some(other)).await;

		let threads = Comment::query()
			.filter(|comment| comment.parent_id.is_null())
			.order_by(|comment| comment.body.desc())
			.shape::<Thread>()
			.all(&db)
			.await
			.unwrap();
		let leaf = |body: &str| Leaf { body: body.into() };
		let reply = |body: &str, parent: &str, replies: &[&str]| {
			Reply {
				body: body.into(),
				replies: replies.iter().map(|body| leaf(body)).collect(),
				parent: Some(leaf(parent)),
			}
		};
		assert_eq!(
			threads,
			[
				Thread {
					body: "root".into(),
					replies: vec![
						reply("a", "root", &["a.1", "a.2"]),
						reply("b", "root", &["b.1"]),
					],
				},
				Thread {
					body: "other root".into(),
					replies: vec![reply("other.1", "other root", &[])],
				},
			]
		);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bypassing_policies_reaches_every_nested_row() {
	on_both(async |db| {
		let ada = author(&db, "ada").await;
		post(&db, &ada, "Draft", false, 1).await;
		let card = |db: Db| {
			async move {
				Author::query()
					.shape::<ProfileCard>()
					.one(&db)
					.await
					.unwrap()
			}
		};

		assert_eq!(card(db.clone()).await.post_count, 0);
		let system = card(db.system("checking every draft")).await;
		assert_eq!(
			(system.post_count, system.drafts, system.top.len()),
			(1, 1, 1)
		);
	})
	.await;
}

/// A world to check, by position.
#[derive(Clone, Debug)]
struct Spec {
	authors: usize,
	/// `(author, published, deleted)`.
	posts: Vec<(usize, bool, bool)>,
	tags: usize,
	/// `(post, tag, private)`, at most once per pair.
	links: Vec<(usize, usize, bool)>,
	viewer: Option<usize>,
}

fn spec() -> impl Strategy<Value = Spec> {
	(1..=3_usize, 0..=5_usize, 1..=3_usize)
		.prop_flat_map(|(authors, posts, tags)| {
			(
				Just(authors),
				proptest::collection::vec(
					(0..authors, any::<bool>(), proptest::bool::weighted(0.2)),
					posts,
				),
				Just(tags),
				proptest::collection::vec((0..posts.max(1), 0..tags, any::<bool>()), 0..8),
				proptest::option::of(0..authors),
			)
		})
		.prop_map(|(authors, posts, tags, mut links, viewer)| {
			let mut seen = Vec::new();
			links.retain(|&(post, tag, _)| {
				let fresh = post < posts.len() && !seen.contains(&(post, tag));
				seen.push((post, tag));
				fresh
			});
			Spec {
				authors,
				posts,
				tags,
				links,
				viewer,
			}
		})
}

/// The rules, in plain Rust.
impl Spec {
	fn post_visible(&self, post: usize) -> bool {
		let (author, published, deleted) = self.posts[post];
		!deleted && (published || self.viewer == Some(author))
	}

	/// Each visible post of `author`, with its visible tags.
	fn posts_of(&self, author: usize) -> Vec<(usize, Vec<usize>)> {
		(0..self.posts.len())
			.filter(|&post| self.posts[post].0 == author && self.post_visible(post))
			.map(|post| {
				let mut tags: Vec<usize> = self
					.links
					.iter()
					.filter(|&&(linked, _, private)| {
						linked == post && (!private || self.viewer == Some(author))
					})
					.map(|&(_, tag, _)| tag)
					.collect();
				tags.sort_unstable();
				(post, tags)
			})
			.collect()
	}
}

/// What the property test fetches: each author's posts, by key, each with
/// its tags, by name.
#[derive(Shape, Debug, PartialEq)]
#[db(from = Author)]
pub struct Model {
	pub handle: String,
	#[db(count(posts))]
	pub posts: i64,
	#[db(via = posts, order_by = |post| post.rank.asc())]
	pub listed: Vec<ModelPost>,
}

#[derive(Shape, Debug, PartialEq)]
#[db(from = Post)]
pub struct ModelPost {
	pub rank: i16,
	#[db(via = tags, order_by = |tag| tag.name.asc())]
	pub tags: Vec<TagName>,
}

async fn seed(db: &Db, spec: &Spec) {
	let system = db.system("seeding the property test");
	PostTag::query().destroy(&system).await.unwrap();
	Vote::query().destroy(&system).await.unwrap();
	Comment::query().destroy(&system).await.unwrap();
	Post::query().destroy(&system).await.unwrap();
	Tag::query().destroy(&system).await.unwrap();
	Author::query().destroy(&system).await.unwrap();

	let mut authors = Vec::new();
	let mut posts = Vec::new();
	let mut tags = Vec::new();

	for index in 0..spec.authors {
		authors.push(author(&system, &format!("author {index}")).await);
	}

	for (index, &(author, published, _)) in spec.posts.iter().enumerate() {
		let rank = i16::try_from(index).unwrap();
		let post = new_post(&authors[author], "post", published, rank)
			.insert(&system)
			.await
			.unwrap();
		posts.push(post);
	}

	for index in 0..spec.tags {
		let tag = NewTag {
			name: format!("tag {index}"),
		};
		tags.push(tag.insert(&system).await.unwrap().id);
	}

	for &(post, tag, private) in &spec.links {
		posts[post]
			.tags()
			.add(&system, tags[tag], PostTagProperties { private })
			.await
			.unwrap();
	}

	for (post, &(_, _, deleted)) in posts.iter_mut().zip(&spec.posts) {
		if deleted {
			post.delete(&system).await.unwrap();
		}
	}
}

async fn agrees_with_the_model(db: &Db, spec: Spec) -> Result<(), TestCaseError> {
	seed(db, &spec).await;
	let system = db.system("finding the viewer");
	let request = match spec.viewer {
		Some(index) => {
			let viewer = Author::query()
				.filter(|author| author.handle.eq(format!("author {index}")))
				.one(&system)
				.await
				.unwrap();
			as_author(db, &viewer)
		}
		None => db.clone(),
	};

	let shaped = Author::query()
		.order_by(|author| author.handle.asc())
		.shape::<Model>()
		.all(&request)
		.await
		.unwrap();
	let expected: Vec<Model> = (0..spec.authors)
		.map(|author| {
			let posts = spec.posts_of(author);
			Model {
				handle: format!("author {author}"),
				posts: posts.len() as i64,
				listed: posts
					.into_iter()
					.map(|(post, tags)| {
						ModelPost {
							rank: i16::try_from(post).unwrap(),
							tags: tags
								.into_iter()
								.map(|tag| {
									TagName {
										name: format!("tag {tag}"),
									}
								})
								.collect(),
						}
					})
					.collect(),
			}
		})
		.collect();
	prop_assert_eq!(shaped, expected);
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
fn shapes_agree_with_the_model_on_sqlite() {
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let db = runtime.block_on(async {
		let db = Db::connect("sqlite::memory:").await.unwrap();
		create_schema(&db).await;
		db
	});
	run_property(&db, &runtime, 96);
}

#[test]
fn shapes_agree_with_the_model_on_postgres() {
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
