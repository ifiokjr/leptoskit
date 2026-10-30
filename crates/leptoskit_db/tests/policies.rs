//! Access policies against both databases: reads filtered silently, writes
//! filtered and then checked, refusals explained, and every refused write
//! rolled back.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::Versioned;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;

/// The signed-in author: a global the policies read.
struct Viewer(Id<Author>);

/// A request from an administrator.
struct Admin;

/// Someone who writes. No policies: open to server code.
#[table(name = "authors")]
pub struct Author {
	pub id: Id<Self>,
	pub name: String,
}

/// An article: everyone reads published ones, authors read and change
/// their own, administrators do anything, and locked articles never change.
#[table(name = "articles")]
#[policy(select, |article, cx| article.published.eq(true)
	| cx.global(|viewer: &Viewer| article.author_id.eq(viewer.0)))]
#[policy(insert, update, delete, |article, cx| cx.global(|viewer: &Viewer| article.author_id.eq(viewer.0)),
	message = "Only the author can change an article")]
#[policy(all, |_, cx| cx.global(|_: &Admin| Expr::always()))]
#[policy(deny update, |article, _| article.locked.eq(true), message = "Locked articles can't change")]
pub struct Article {
	pub id: Id<Self>,
	pub author_id: Id<Author>,
	#[db(unique(except = deleted_at))]
	pub slug: String,
	pub published: bool,
	pub locked: bool,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
	#[db(flatten)]
	pub version: Versioned,
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

/// Two authors, each with a published article and a draft, plus a locked
/// article by Ada.
struct World {
	ada: Db,
	grace: Db,
	anonymous: Db,
	admin: Db,
	system: Db,
	ada_id: Id<Author>,
	grace_id: Id<Author>,
}

async fn world(db: Db) -> World {
	let system = db.system("seeding the test");
	let ada = NewAuthor { name: "Ada".into() }
		.insert(&system)
		.await
		.unwrap();
	let grace = NewAuthor {
		name: "Grace".into(),
	}
	.insert(&system)
	.await
	.unwrap();

	for (author, slug, published, locked) in [
		(ada.id, "ada-live", true, false),
		(ada.id, "ada-draft", false, false),
		(ada.id, "ada-locked", true, true),
		(grace.id, "grace-live", true, false),
		(grace.id, "grace-draft", false, false),
	] {
		NewArticle {
			author_id: author,
			slug: slug.into(),
			published,
			locked,
		}
		.insert(&system)
		.await
		.unwrap();
	}

	World {
		ada: db.with_context(Context::new().with(Viewer(ada.id))),
		grace: db.with_context(Context::new().with(Viewer(grace.id))),
		anonymous: db.clone(),
		admin: db.with_context(Context::new().with(Admin)),
		system,
		ada_id: ada.id,
		grace_id: grace.id,
	}
}

async fn slugs(db: &Db) -> Vec<String> {
	let mut slugs: Vec<String> = Article::query()
		.all(db)
		.await
		.unwrap()
		.into_iter()
		.map(|article| article.slug)
		.collect();
	slugs.sort();
	slugs
}

async fn article(world: &World, slug: &str) -> Article {
	Article::query()
		.filter(|article| article.slug.eq(slug))
		.one(&world.system)
		.await
		.unwrap()
}

fn message(error: &DbError) -> String {
	assert!(error.is_forbidden(), "{error:?}");
	error.to_string()
}

#[test]
fn contexts_hold_one_global_per_type() {
	let cx = Context::new().with(Viewer(Id::new())).with(Admin);
	assert!(cx.get::<Viewer>().is_some());
	assert!(cx.get::<Admin>().is_some());
	assert!(Context::new().get::<Viewer>().is_none());
	assert_eq!(format!("{cx:?}"), "Context { globals: 2 }");

	let first = Id::new();
	let second = Id::new();
	let replaced = Context::new().with(Viewer(first)).with(Viewer(second));
	assert_eq!(replaced.get::<Viewer>().unwrap().0, second);
	assert_eq!(
		Context::new().global(|_: &Admin| Expr::always()),
		Expr::never()
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn reads_show_only_what_each_request_may_see() {
	on_both(async |db| {
		let world = world(db).await;
		let published = ["ada-live", "ada-locked", "grace-live"];

		assert_eq!(slugs(&world.anonymous).await, published);
		assert_eq!(
			slugs(&world.ada).await,
			["ada-draft", "ada-live", "ada-locked", "grace-live"]
		);
		assert_eq!(slugs(&world.admin).await.len(), 5);
		assert_eq!(slugs(&world.system).await.len(), 5);

		let draft = article(&world, "grace-draft").await;
		assert!(Article::find(&world.ada, draft.id).await.unwrap().is_none());
		assert_eq!(Article::query().count(&world.anonymous).await.unwrap(), 3);
		assert!(
			!Article::query()
				.filter(|a| a.slug.eq("ada-draft"))
				.exists(&world.grace)
				.await
				.unwrap()
		);
		assert!(world.anonymous.context().is_some() && world.system.context().is_none());

		// A transaction acts for whoever began it.
		let seen = world
			.ada
			.transaction(async |tx| Article::query().count(tx).await)
			.await
			.unwrap();
		assert_eq!(seen, 4);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn inserts_must_be_allowed_as_stored() {
	on_both(async |db| {
		let world = world(db).await;
		let mine = NewArticle {
			author_id: world.ada_id,
			slug: "ada-new".into(),
			published: false,
			locked: false,
		}
		.insert(&world.ada)
		.await
		.unwrap();
		assert_eq!(mine.author_id, world.ada_id);

		let impostor = NewArticle {
			author_id: world.grace_id,
			slug: "forged".into(),
			published: true,
			locked: false,
		}
		.insert(&world.ada)
		.await
		.unwrap_err();
		assert_eq!(message(&impostor), "Only the author can change an article");

		let anonymous = NewArticle {
			author_id: world.ada_id,
			slug: "nobody".into(),
			published: true,
			locked: false,
		}
		.insert(&world.anonymous)
		.await
		.unwrap_err();
		assert!(anonymous.is_forbidden());

		// Refused inserts roll back.
		assert_eq!(slugs(&world.system).await.len(), 6);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn saves_need_the_row_and_its_result_to_be_allowed() {
	on_both(async |db| {
		let world = world(db).await;

		let mut draft = Article::query()
			.filter(|article| article.slug.eq("ada-draft"))
			.one(&world.ada)
			.await
			.unwrap();
		draft.published = true;
		draft.save(&world.ada).await.unwrap();

		// Giving it away is a result Ada may not write.
		draft.author_id = world.grace_id;
		let given = draft.save(&world.ada).await.unwrap_err();
		assert_eq!(message(&given), "Only the author can change an article");
		assert_eq!(article(&world, "ada-draft").await.author_id, world.ada_id);

		// Grace can see Ada's published article but not change it.
		let mut theirs = article(&world, "ada-live").await;
		theirs.published = false;
		let refused = theirs.save(&world.grace).await.unwrap_err();
		assert_eq!(message(&refused), "Only the author can change an article");

		// A row the request can't see isn't found, rather than forbidden.
		let mut hidden = article(&world, "grace-draft").await;
		hidden.published = true;
		assert!(hidden.save(&world.ada).await.unwrap_err().is_not_found());

		// `deny` wins, even for the author and an administrator.
		let mut locked = article(&world, "ada-locked").await;
		locked.published = false;
		assert_eq!(
			message(&locked.save(&world.ada).await.unwrap_err()),
			"Locked articles can't change"
		);
		assert!(locked.save(&world.admin).await.unwrap_err().is_forbidden());
		locked.save(&world.system).await.unwrap();
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bulk_updates_skip_what_they_may_not_change() {
	on_both(async |db| {
		let world = world(db).await;

		let changed = Article::update()
			.set(|article| article.published, true)
			.run(&world.grace)
			.await
			.unwrap();
		assert_eq!(changed, 2, "only Grace's own articles");
		assert_eq!(
			Article::update()
				.set(|article| article.published, true)
				.run(&world.anonymous)
				.await
				.unwrap(),
			0
		);

		let stolen = Article::update()
			.filter(|article| article.slug.eq("grace-live"))
			.set(|article| article.author_id, world.ada_id)
			.run(&world.grace)
			.await
			.unwrap_err();
		assert!(stolen.is_forbidden());
		assert_eq!(
			article(&world, "grace-live").await.author_id,
			world.grace_id
		);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn deletes_need_permission_too() {
	on_both(async |db| {
		let world = world(db).await;

		let mut theirs = article(&world, "ada-live").await;
		assert!(
			theirs
				.delete(&world.grace)
				.await
				.unwrap_err()
				.is_forbidden()
		);
		assert!(article(&world, "ada-live").await.deleted_at.is_none());

		let deleted = Article::query().delete(&world.grace).await.unwrap();
		assert_eq!(deleted, 2);
		assert_eq!(slugs(&world.anonymous).await, ["ada-live", "ada-locked"]);

		let hidden = Article::query()
			.with_deleted()
			.filter(|article| article.slug.eq("grace-draft"))
			.one(&world.system)
			.await
			.unwrap();
		assert!(hidden.destroy(&world.ada).await.unwrap_err().is_not_found());
		assert_eq!(
			Article::query()
				.with_deleted()
				.destroy(&world.ada)
				.await
				.unwrap(),
			3
		);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn upserts_cannot_take_over_rows() {
	on_both(async |db| {
		let world = world(db).await;

		let takeover = NewArticle {
			author_id: world.ada_id,
			slug: "grace-live".into(),
			published: true,
			locked: false,
		}
		.upsert(&world.ada, |article| article.slug)
		.await
		.unwrap_err();
		assert_eq!(message(&takeover), "Only the author can change an article");
		assert_eq!(
			article(&world, "grace-live").await.author_id,
			world.grace_id
		);

		let fresh = NewArticle {
			author_id: world.ada_id,
			slug: "ada-upserted".into(),
			published: false,
			locked: false,
		}
		.upsert(&world.ada, |article| article.slug)
		.await
		.unwrap();
		assert_eq!(fresh.slug, "ada-upserted");

		let again = NewArticle {
			author_id: world.ada_id,
			slug: "ada-upserted".into(),
			published: true,
			locked: false,
		}
		.upsert(&world.ada, |article| article.slug)
		.await
		.unwrap();
		assert_eq!(again.id, fresh.id);
		assert!(again.published);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tables_without_policies_stay_open() {
	on_both(async |db| {
		let author = NewAuthor {
			name: "Open".into(),
		}
		.insert(&db)
		.await
		.unwrap();
		assert_eq!(Author::get(&db, author.id).await.unwrap().name, "Open");
		assert_eq!(Author::query().count(&db).await.unwrap(), 1);
		let _ = db.dialect();
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn refused_and_stale_saves_are_told_apart() {
	on_both(async |db| {
		let world = world(db).await;

		// A row someone else changed is stale, even under policies.
		let mut first = article(&world, "ada-draft").await;
		let mut second = article(&world, "ada-draft").await;
		first.published = true;
		first.save(&world.ada).await.unwrap();
		second.published = false;
		assert!(second.save(&world.ada).await.unwrap_err().is_stale());

		// A current row the request may not change is forbidden, not stale.
		let mut theirs = article(&world, "grace-live").await;
		theirs.published = false;
		assert!(theirs.save(&world.ada).await.unwrap_err().is_forbidden());
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_join_the_callers_transaction_and_its_checks() {
	on_both(async |db| {
		let world = world(db).await;
		let ada_id = world.ada_id;
		let grace_id = world.grace_id;

		let outcome = world
			.ada
			.transaction(async |tx| {
				let mut article = NewArticle {
					author_id: ada_id,
					slug: "in-tx".into(),
					published: false,
					locked: false,
				}
				.insert(tx)
				.await?;
				article.published = true;
				article.save(tx).await?;
				NewArticle {
					author_id: ada_id,
					slug: "in-tx".into(),
					published: false,
					locked: false,
				}
				.upsert(tx, |article| article.slug)
				.await?;
				Article::update()
					.filter(|article| article.slug.eq("in-tx"))
					.set(|article| article.locked, true)
					.run(tx)
					.await?;

				// A refusal inside the transaction fails the whole transaction.
				Article::update()
					.filter(|article| article.slug.eq("in-tx"))
					.set(|article| article.author_id, grace_id)
					.run(tx)
					.await
			})
			.await;
		assert!(outcome.unwrap_err().is_forbidden());

		// Rolled back together: nothing from the transaction remains.
		assert!(
			Article::query()
				.filter(|article| article.slug.eq("in-tx"))
				.first(&world.system)
				.await
				.unwrap()
				.is_none()
		);
	})
	.await;
}
