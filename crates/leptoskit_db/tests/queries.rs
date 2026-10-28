//! Queries and writes against real databases: SQLite in memory, and a
//! throwaway Postgres cluster. Both run the same checks.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use leptoskit_db::Connection;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Dialect;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::Timestamp;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;

/// A level of trust.
#[derive(leptoskit_db::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
	Member,
	Admin,
}

#[table(
	name = "users",
	check(name = "users_karma_check", sql = "karma > -1000")
)]
pub struct User {
	pub id: Id<Self>,
	#[db(unique(except = deleted_at), max_len = 32)]
	pub handle: String,
	pub name: Option<String>,
	pub role: Role,
	pub karma: i32,
	pub tags: Json<Vec<String>>,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

#[table(name = "posts", index(author_id))]
pub struct Post {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub author_id: Id<User>,
	pub title: String,
	pub score: i64,
	pub published_on: Option<chrono::NaiveDate>,
}

/// Keyed by its two columns; no `Id`.
#[table(name = "follows")]
pub struct Follow {
	#[db(key)]
	pub follower_id: Id<User>,
	#[db(key)]
	pub followed_id: Id<User>,
	pub muted: bool,
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

fn new_user(handle: &str) -> NewUser {
	NewUser {
		handle: handle.into(),
		name: None,
		role: Role::Member,
		karma: 0,
		tags: Json(vec!["new".into()]),
	}
}

async fn sqlite() -> Db {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	create_schema(&db).await;
	db
}

/// Runs `check` against SQLite, then Postgres when it's available.
async fn on_both<F, Fut>(check: F)
where
	F: Fn(Db) -> Fut,
	Fut: Future<Output = ()>,
{
	check(sqlite().await).await;

	if let Some(cluster) = support::postgres::PostgresCluster::start() {
		let db = Db::connect(&cluster.database()).await.unwrap();
		assert_eq!(db.dialect(), Dialect::Postgres);
		create_schema(&db).await;
		check(db).await;
	}
}

#[tokio::test(flavor = "multi_thread")]
async fn inserts_read_back_exactly() {
	on_both(async |db| {
		let user = new_user("ada").insert(&db).await.unwrap();
		let found = User::get(&db, user.id).await.unwrap();
		assert_eq!(format!("{found:?}"), format!("{user:?}"));
		assert_eq!(found.tags, Json(vec!["new".to_string()]));
		assert!(User::find(&db, Id::new()).await.unwrap().is_none());
		assert!(User::get(&db, Id::new()).await.unwrap_err().is_not_found());

		let post = NewPost {
			author_id: user.id,
			title: "Hello".into(),
			score: i64::MAX,
			published_on: chrono::NaiveDate::from_ymd_opt(2026, 10, 28),
		}
		.insert(&db)
		.await
		.unwrap();
		assert_eq!(Post::get(&db, post.id).await.unwrap().score, i64::MAX);
	})
	.await;
}

/// Four users with different karma; the three with more than 15 have names.
async fn seed_people(db: &Db) {
	for (handle, karma) in [("ada", 10), ("grace", 30), ("alan", 20), ("barbara", 40)] {
		let mut user = new_user(handle);
		user.karma = karma;
		user.name = (karma > 15).then(|| handle.to_uppercase());
		user.insert(db).await.unwrap();
	}
}

fn handles(users: Vec<User>) -> Vec<String> {
	users.into_iter().map(|user| user.handle).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn queries_order_and_page() {
	on_both(async |db| {
		seed_people(&db).await;
		let ordered = User::query()
			.order_by(|u| u.karma.desc())
			.all(&db)
			.await
			.unwrap();
		assert_eq!(handles(ordered), ["barbara", "grace", "alan", "ada"]);

		let page = User::query()
			.order_by(|u| u.karma.asc())
			.offset(1)
			.limit(2)
			.all(&db)
			.await
			.unwrap();
		assert_eq!(handles(page), ["alan", "grace"]);
		let rest = User::query()
			.order_by(|u| u.karma.asc())
			.offset(3)
			.all(&db)
			.await
			.unwrap();
		assert_eq!(handles(rest), ["barbara"]);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn filters_combine_and_match_text() {
	on_both(async |db| {
		seed_people(&db).await;
		let middle = User::query()
			.filter(|u| u.karma.gt(10) & u.karma.lte(30))
			.order_by(|u| u.handle.asc());
		assert_eq!(handles(middle.all(&db).await.unwrap()), ["alan", "grace"]);

		let either = User::query().filter(|u| u.handle.eq("ada") | u.handle.eq("barbara"));
		assert_eq!(either.count(&db).await.unwrap(), 2);
		assert_eq!(
			User::query()
				.filter(|u| !u.karma.gte(20))
				.count(&db)
				.await
				.unwrap(),
			1
		);
		assert_eq!(
			User::query()
				.filter(|u| u.karma.lt(20) & u.karma.ne(10))
				.count(&db)
				.await
				.unwrap(),
			0
		);
		assert_eq!(
			User::query()
				.filter(|u| u.name.is_null())
				.count(&db)
				.await
				.unwrap(),
			1
		);
		assert_eq!(
			User::query()
				.filter(|u| u.name.is_not_null())
				.count(&db)
				.await
				.unwrap(),
			3
		);
		assert_eq!(
			User::query()
				.filter(|u| u.name.eq(None::<String>))
				.count(&db)
				.await
				.unwrap(),
			1
		);
		assert_eq!(
			User::query()
				.filter(|u| u.name.ne(None::<String>))
				.count(&db)
				.await
				.unwrap(),
			3
		);
		assert_eq!(
			User::query()
				.filter(|u| u.name.eq("GRACE"))
				.count(&db)
				.await
				.unwrap(),
			1
		);
		assert_eq!(
			User::query()
				.filter(|u| u.handle.in_list(["ada", "alan", "nobody"]))
				.count(&db)
				.await
				.unwrap(),
			2
		);
		assert_eq!(
			User::query()
				.filter(|u| u.handle.in_list(Vec::<String>::new()))
				.count(&db)
				.await
				.unwrap(),
			0
		);
		assert_eq!(
			User::query()
				.filter(|u| u.handle.contains("ar"))
				.count(&db)
				.await
				.unwrap(),
			1
		);
		assert_eq!(
			User::query()
				.filter(|u| u.handle.contains("AR"))
				.count(&db)
				.await
				.unwrap(),
			0
		);
		assert_eq!(
			User::query()
				.filter(|u| u.handle.ilike("A%"))
				.count(&db)
				.await
				.unwrap(),
			2
		);
		assert_eq!(
			User::query()
				.filter(|u| u.role.eq(Role::Member))
				.count(&db)
				.await
				.unwrap(),
			4
		);
		assert!(
			User::query()
				.filter(|u| u.handle.eq("grace"))
				.exists(&db)
				.await
				.unwrap()
		);
		assert!(
			!User::query()
				.filter(|u| u.handle.eq("nobody"))
				.exists(&db)
				.await
				.unwrap()
		);
		assert!(
			User::query()
				.filter(|u| u.handle.eq("nobody"))
				.one(&db)
				.await
				.unwrap_err()
				.is_not_found()
		);

		let ada = User::query()
			.filter(|u| u.handle.eq("ada"))
			.one(&db)
			.await
			.unwrap();
		assert_eq!(
			User::query()
				.filter(|u| u.handle.eq(&ada.handle))
				.count(&db)
				.await
				.unwrap(),
			1
		);
		assert_eq!(ada.handle, "ada", "comparing borrows the value");
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn saves_bump_updated_at_and_keep_created_at() {
	on_both(async |db| {
		let mut user = new_user("ada").insert(&db).await.unwrap();
		let created = user.created_at;
		tokio::time::sleep(std::time::Duration::from_millis(5)).await;

		user.karma = 5;
		user.role = Role::Admin;
		user.save(&db).await.unwrap();

		let stored = User::get(&db, user.id).await.unwrap();
		assert_eq!((stored.karma, stored.role), (5, Role::Admin));
		assert_eq!(stored.created_at, created);
		assert!(stored.updated_at > created);
		assert_eq!(stored.updated_at, user.updated_at);

		let mut ghost = new_user("ghost").into_row();
		assert!(ghost.save(&db).await.unwrap_err().is_not_found());
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn soft_deletes_hide_rows_until_asked() {
	on_both(async |db| {
		let mut ada = new_user("ada").insert(&db).await.unwrap();
		new_user("grace").insert(&db).await.unwrap();

		ada.delete(&db).await.unwrap();
		assert!(ada.deleted_at.is_some());
		assert!(User::find(&db, ada.id).await.unwrap().is_none());
		assert_eq!(User::query().count(&db).await.unwrap(), 1);
		assert_eq!(User::query().with_deleted().count(&db).await.unwrap(), 2);

		// The handle frees up: the unique index skips soft-deleted rows.
		new_user("ada").insert(&db).await.unwrap();

		assert_eq!(
			User::query()
				.filter(|u| u.handle.eq("grace"))
				.delete(&db)
				.await
				.unwrap(),
			1
		);
		assert_eq!(User::query().count(&db).await.unwrap(), 1);
		assert_eq!(User::query().with_deleted().destroy(&db).await.unwrap(), 3);

		let post_author = new_user("author").insert(&db).await.unwrap();
		let post = NewPost {
			author_id: post_author.id,
			title: "t".into(),
			score: 1,
			published_on: None,
		}
		.insert(&db)
		.await
		.unwrap();
		post.destroy(&db).await.unwrap();
		assert_eq!(Post::query().count(&db).await.unwrap(), 0);

		let mut gone = NewPost {
			author_id: post_author.id,
			title: "t".into(),
			score: 1,
			published_on: None,
		}
		.insert(&db)
		.await
		.unwrap();
		gone.delete(&db).await.unwrap();
		assert_eq!(
			Post::query().with_deleted().count(&db).await.unwrap(),
			0,
			"no soft delete column"
		);
		assert_eq!(Post::query().delete(&db).await.unwrap(), 0);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bulk_updates_set_columns_and_updated_at() {
	on_both(async |db| {
		let before = new_user("ada").insert(&db).await.unwrap();
		new_user("grace").insert(&db).await.unwrap();
		tokio::time::sleep(std::time::Duration::from_millis(5)).await;

		let changed = User::update()
			.filter(|u| u.handle.eq("ada"))
			.set(|u| u.karma, 7)
			.set(|u| u.role, Role::Admin)
			.run(&db)
			.await
			.unwrap();
		assert_eq!(changed, 1);

		let after = User::get(&db, before.id).await.unwrap();
		assert_eq!((after.karma, after.role), (7, Role::Admin));
		assert!(after.updated_at > before.updated_at);
		assert_eq!(
			User::update().set(|u| u.karma, 1).run(&db).await.unwrap(),
			2
		);
		assert_eq!(
			User::update()
				.with_deleted()
				.set(|u| u.name, Some("x".to_string()))
				.run(&db)
				.await
				.unwrap(),
			2
		);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn upserts_update_the_matching_row() {
	on_both(async |db| {
		let first = new_user("ada").insert(&db).await.unwrap();
		let mut again = new_user("ada");
		again.karma = 99;
		let merged = again.upsert(&db, |u| u.handle).await.unwrap();

		assert_eq!(merged.id, first.id, "the existing key is kept");
		assert_eq!(merged.created_at, first.created_at);
		assert_eq!(merged.karma, 99);
		assert_eq!(User::query().count(&db).await.unwrap(), 1);

		let grace = new_user("grace").insert(&db).await.unwrap();
		let follow = NewFollow {
			follower_id: first.id,
			followed_id: grace.id,
			muted: false,
		};
		follow
			.upsert(&db, |f| (f.follower_id, f.followed_id))
			.await
			.unwrap();
		let muted = NewFollow {
			follower_id: first.id,
			followed_id: grace.id,
			muted: true,
		}
		.upsert(&db, |f| (f.follower_id, f.followed_id))
		.await
		.unwrap();
		assert!(muted.muted);
		assert_eq!(Follow::query().count(&db).await.unwrap(), 1);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn broken_rules_are_typed_errors() {
	on_both(async |db| {
		let ada = new_user("ada").insert(&db).await.unwrap();

		let duplicate = new_user("ada").insert(&db).await.unwrap_err();
		assert!(duplicate.is_conflict(), "{duplicate:?}");
		assert_eq!(
			duplicate.to_string(),
			"a `users` row with the same unique value already exists"
		);

		let orphan = NewPost {
			author_id: Id::new(),
			title: "t".into(),
			score: 0,
			published_on: None,
		};
		let error = orphan.insert(&db).await.unwrap_err();
		assert!(matches!(error, DbError::ForeignKey { .. }), "{error:?}");

		let mut cold = new_user("cold");
		cold.karma = -5000;
		let error = cold.insert(&db).await.unwrap_err();
		assert!(matches!(&error, DbError::Check { .. }), "{error:?}");
		assert!(
			!error.to_string().contains("5000"),
			"no values in messages: {error}"
		);

		let too_long = new_user(&"x".repeat(40)).insert(&db).await.unwrap_err();
		assert!(matches!(too_long, DbError::Invalid(_)));
		assert_eq!(
			too_long.to_string(),
			"`handle` can have at most 32 characters"
		);

		// Deleting a user cascades to their posts.
		let author = ada;
		NewPost {
			author_id: author.id,
			title: "t".into(),
			score: 0,
			published_on: None,
		}
		.insert(&db)
		.await
		.unwrap();
		author.destroy(&db).await.unwrap();
		assert_eq!(Post::query().count(&db).await.unwrap(), 0);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn transactions_commit_or_roll_back_together() {
	on_both(async |db| {
		let committed = db
			.transaction(async |tx| {
				let user = new_user("ada").insert(tx).await?;
				NewPost {
					author_id: user.id,
					title: "t".into(),
					score: 0,
					published_on: None,
				}
				.insert(tx)
				.await?;
				assert_eq!(
					User::query().count(tx).await?,
					1,
					"a transaction sees its own writes"
				);
				Ok::<_, DbError>(user)
			})
			.await
			.unwrap();
		assert!(User::find(&db, committed.id).await.unwrap().is_some());

		let failed = db
			.transaction(async |tx| {
				new_user("grace").insert(tx).await?;
				new_user("ada").insert(tx).await?;
				Ok::<_, DbError>(())
			})
			.await;
		assert!(failed.unwrap_err().is_conflict());
		assert_eq!(
			User::query().count(&db).await.unwrap(),
			1,
			"grace was rolled back"
		);

		let explicit = db.begin().await.unwrap();
		new_user("alan").insert(&explicit).await.unwrap();
		explicit.rollback().await.unwrap();

		let abandoned = db.begin().await.unwrap();
		new_user("barbara").insert(&abandoned).await.unwrap();
		drop(abandoned);
		assert_eq!(
			User::query().count(&db).await.unwrap(),
			1,
			"dropped transactions roll back"
		);

		let explicit = db.begin().await.unwrap();
		new_user("edsger").insert(&explicit).await.unwrap();
		explicit.commit().await.unwrap();
		assert_eq!(User::query().count(&db).await.unwrap(), 2);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn transactions_can_run_in_spawned_tasks() {
	fn assert_send<T: Send>(value: T) -> T {
		value
	}

	let db = sqlite().await;
	let task = tokio::spawn(assert_send(async move {
		db.transaction(async |tx| new_user("ada").insert(tx).await)
			.await
			.unwrap();
		User::query().count(&db).await.unwrap()
	}));
	assert_eq!(task.await.unwrap(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn files_and_bad_urls() {
	let directory = tempfile::tempdir().unwrap();
	let path = directory.path().join("app.sqlite3");
	let db = Db::connect(&format!("sqlite://{}", path.display()))
		.await
		.unwrap();
	create_schema(&db).await;
	new_user("ada").insert(&db).await.unwrap();

	let reopened = Db::connect(&format!("sqlite:{}", path.display()))
		.await
		.unwrap();
	assert_eq!(User::query().count(&reopened).await.unwrap(), 1);

	let unsupported = Db::connect("mysql://localhost/app").await.err().unwrap();
	assert!(matches!(unsupported, DbError::Connection(_)));
	assert_eq!(unsupported.to_string(), "the database can't be reached");

	let missing = Db::connect("sqlite:///no/such/directory/app.sqlite3")
		.await
		.err()
		.unwrap();
	assert!(matches!(missing, DbError::Connection(_)));
}

/// One column of every primitive type.
#[table(name = "samples")]
pub struct Sample {
	pub id: Id<Self>,
	pub small: i16,
	pub tiny: u8,
	pub ratio: f32,
	pub precise: f64,
	pub flag: bool,
	pub bytes: Vec<u8>,
	pub external: uuid::Uuid,
	pub huge: u64,
	pub data: Option<Json<serde_json::Value>>,
	pub seen_at: Option<Timestamp>,
}

#[tokio::test(flavor = "multi_thread")]
async fn every_primitive_round_trips_through_both_drivers() {
	on_both(async |db| {
		let sample = NewSample {
			small: -7,
			tiny: 255,
			ratio: 0.5,
			precise: -1.25,
			flag: true,
			bytes: vec![0, 1, 255],
			external: uuid::Uuid::now_v7(),
			huge: u64::from(u32::MAX) + 1,
			data: Some(Json(serde_json::json!({ "nested": [1, 2] }))),
			seen_at: Some(leptoskit_db::now()),
		}
		.insert(&db)
		.await
		.unwrap();
		let stored = Sample::get(&db, sample.id).await.unwrap();
		assert_eq!(format!("{stored:?}"), format!("{sample:?}"));

		assert_eq!(
			Sample::query()
				.filter(|s| s.small.lt(0) & s.ratio.gt(0.25))
				.count(&db)
				.await
				.unwrap(),
			1
		);
		assert_eq!(
			Sample::query()
				.filter(|s| s.tiny.eq(255) & s.flag.eq(true))
				.count(&db)
				.await
				.unwrap(),
			1
		);

		let too_big = Sample::query()
			.filter(|s| s.huge.eq(u64::MAX))
			.count(&db)
			.await
			.unwrap_err();
		assert!(matches!(too_big, DbError::Encode(_)), "{too_big:?}");
		let null_order = Sample::query()
			.filter(|s| s.seen_at.gt(None::<Timestamp>))
			.all(&db)
			.await
			.unwrap_err();
		assert!(matches!(null_order, DbError::Encode(_)), "{null_order:?}");
		let bad_set = Sample::update()
			.set(|s| s.huge, u64::MAX)
			.run(&db)
			.await
			.unwrap_err();
		assert!(matches!(bad_set, DbError::Encode(_)), "{bad_set:?}");
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tls_is_required_when_the_url_asks_for_it() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};
	let url = cluster
		.database()
		.replace("sslmode=disable", "sslmode=require");
	let db = Db::connect(&url).await.unwrap();
	let error = Sample::query().count(&db).await.unwrap_err();
	assert!(
		matches!(error, DbError::Connection(_)),
		"the test cluster has no TLS: {error:?}"
	);
}
