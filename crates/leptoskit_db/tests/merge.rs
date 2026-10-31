//! Moving an account's rows by each table's merge strategy, against both
//! databases.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use std::future::Future;

use leptoskit_db::Connection;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::merge;
use leptoskit_db::merge::MergeStrategy;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use leptoskit_db_schema::TableName;

/// Stands in for sign-in's accounts.
#[table(name = "accounts", namespace = "people")]
pub struct Account {
	pub id: Id<Self>,
}

/// The app's user, keyed by the account.
#[table(name = "members")]
pub struct Member {
	#[db(on_delete = cascade)]
	pub id: Id<Account>,
	pub name: String,
}

/// Plain rows: reassigned.
#[table(name = "notes")]
pub struct Note {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub author_id: Id<Account>,
}

/// One per pair: the survivor's duplicates win.
#[table(name = "likes", unique(member_id, topic), on_account_merge = reassign_skip_conflicts)]
pub struct Like {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub member_id: Id<Account>,
	pub topic: String,
}

/// Two unique constraints on the reference: a row that would break either
/// is dropped.
#[table(
	name = "seats",
	unique(member_id, row),
	unique(member_id, code),
	on_account_merge = reassign_skip_conflicts
)]
pub struct Seat {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub member_id: Id<Account>,
	pub row: i32,
	pub code: String,
}

/// One per account: the newer one wins.
#[table(name = "prefs", on_account_merge = keep_newest)]
pub struct Pref {
	pub id: Id<Self>,
	#[db(unique, on_delete = cascade)]
	pub member_id: Id<Account>,
	pub theme: String,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// Thrown away.
#[table(name = "drafts", on_account_merge = keep_survivor)]
pub struct Draft {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub member_id: Id<Account>,
}

/// Moved by the app.
#[table(name = "wallets_owned", on_account_merge = custom)]
pub struct Owned {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub member_id: Id<Account>,
}

fn accounts() -> TableName {
	TableName::namespaced("people", "accounts")
}

async fn create_schema(db: &Db) {
	let dialect = db.dialect();
	let schema = leptoskit_db::schema(dialect)
		.unwrap()
		.without_rename_hints();
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
	check(db.system("tests")).await;

	if let Some(cluster) = support::postgres::PostgresCluster::start() {
		let db = Db::connect(&cluster.database()).await.unwrap();
		create_schema(&db).await;
		check(db.system("tests")).await;
	}
}

async fn person(db: &Db, name: &str) -> Id<Account> {
	let account = NewAccount {}.insert(db).await.unwrap();
	NewMember {
		id: account.id,
		name: name.into(),
	}
	.insert(db)
	.await
	.unwrap();
	account.id
}

#[test]
fn the_schema_knows_every_table_that_holds_an_account() {
	let schema = leptoskit_db::schema(leptoskit_db::Dialect::Postgres).unwrap();
	let data = merge::account_data(&schema, &accounts());
	assert_eq!(data.keyed.len(), 1);
	assert_eq!(data.keyed[0].table, TableName::new("members"));
	assert!(data.is_covered(), "{:?}", data.problems);
	assert!(data.has_custom());
	let strategies: Vec<(String, MergeStrategy)> = data
		.references
		.iter()
		.map(|reference| (reference.table.to_string(), reference.strategy))
		.collect();
	assert_eq!(
		strategies,
		[
			("drafts".into(), MergeStrategy::KeepSurvivor),
			("likes".into(), MergeStrategy::ReassignSkipConflicts),
			("notes".into(), MergeStrategy::Reassign),
			("prefs".into(), MergeStrategy::KeepNewest),
			("seats".into(), MergeStrategy::ReassignSkipConflicts),
			("wallets_owned".into(), MergeStrategy::Custom),
		]
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn each_strategy_moves_rows_its_own_way() {
	on_both(|db| {
		async move {
			let survivor = person(&db, "Ada").await;
			let absorbed = person(&db, "Ada (old)").await;

			for author in [survivor, absorbed, absorbed] {
				NewNote { author_id: author }.insert(&db).await.unwrap();
			}

			for (member, topic) in [(survivor, "rust"), (absorbed, "rust"), (absorbed, "solana")] {
				NewLike {
					member_id: member,
					topic: topic.into(),
				}
				.insert(&db)
				.await
				.unwrap();
			}

			NewPref {
				member_id: survivor,
				theme: "light".into(),
			}
			.insert(&db)
			.await
			.unwrap();
			tokio::time::sleep(std::time::Duration::from_millis(5)).await;
			NewPref {
				member_id: absorbed,
				theme: "dark".into(),
			}
			.insert(&db)
			.await
			.unwrap();
			NewDraft {
				member_id: absorbed,
			}
			.insert(&db)
			.await
			.unwrap();

			// The survivor's seat is row 1, code A: the first two of these
			// collide with it, one on each constraint.
			for (member, row, code) in [
				(survivor, 1, "A"),
				(absorbed, 1, "B"),
				(absorbed, 2, "A"),
				(absorbed, 3, "C"),
			] {
				NewSeat {
					member_id: member,
					row,
					code: code.into(),
				}
				.insert(&db)
				.await
				.unwrap();
			}
			NewOwned {
				member_id: absorbed,
			}
			.insert(&db)
			.await
			.unwrap();

			let schema = leptoskit_db::schema(db.dialect()).unwrap();
			let data = merge::account_data(&schema, &accounts());
			let counts: Vec<u64> = futures_counts(&db, &data, absorbed).await;
			assert_eq!(counts, [1, 2, 2, 1, 3, 1]);

			let tx = db.begin().await.unwrap();

			for reference in &data.references {
				merge::move_references(&tx, reference, absorbed.uuid(), survivor.uuid())
					.await
					.unwrap();
			}

			tx.commit().await.unwrap();

			let notes = Note::query()
				.filter(|note| note.author_id.eq(survivor))
				.count(&db)
				.await
				.unwrap();
			assert_eq!(notes, 3);
			let mut topics: Vec<String> = Like::query()
				.filter(|like| like.member_id.eq(survivor))
				.all(&db)
				.await
				.unwrap()
				.into_iter()
				.map(|like| like.topic)
				.collect();
			topics.sort();
			assert_eq!(topics, ["rust", "solana"]);
			let prefs = Pref::query().all(&db).await.unwrap();
			assert_eq!(prefs.len(), 1);
			assert_eq!(prefs[0].member_id, survivor);
			assert_eq!(prefs[0].theme, "dark");
			assert_eq!(Draft::query().count(&db).await.unwrap(), 0);
			// `custom` is the app's to move.
			let owned = Owned::query().all(&db).await.unwrap();
			assert_eq!(owned[0].member_id, absorbed);
			assert_eq!(
				futures_counts(&db, &data, absorbed).await,
				[0, 0, 0, 0, 0, 1]
			);
			let mut seats: Vec<(i32, String)> = Seat::query()
				.filter(|seat| seat.member_id.eq(survivor))
				.all(&db)
				.await
				.unwrap()
				.into_iter()
				.map(|seat| (seat.row, seat.code))
				.collect();
			seats.sort();
			assert_eq!(seats, [(1, "A".to_owned()), (3, "C".to_owned())]);
		}
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_newer_survivor_row_stays_and_conflicts_stop_a_plain_reassign() {
	on_both(|db| {
		async move {
			let absorbed = person(&db, "Old").await;
			let survivor = person(&db, "New").await;
			NewPref {
				member_id: absorbed,
				theme: "dark".into(),
			}
			.insert(&db)
			.await
			.unwrap();
			tokio::time::sleep(std::time::Duration::from_millis(5)).await;
			NewPref {
				member_id: survivor,
				theme: "light".into(),
			}
			.insert(&db)
			.await
			.unwrap();

			let schema = leptoskit_db::schema(db.dialect()).unwrap();
			let data = merge::account_data(&schema, &accounts());
			let prefs = data
				.references
				.iter()
				.find(|reference| reference.table.name == "prefs")
				.unwrap();
			merge::move_references(&db, prefs, absorbed.uuid(), survivor.uuid())
				.await
				.unwrap();
			let left = Pref::query().all(&db).await.unwrap();
			assert_eq!(left.len(), 1);
			assert_eq!(left[0].theme, "light");

			// Without skipping, a duplicate stops the move.
			for member in [survivor, absorbed] {
				NewLike {
					member_id: member,
					topic: "rust".into(),
				}
				.insert(&db)
				.await
				.unwrap();
			}

			let mut likes = data
				.references
				.iter()
				.find(|reference| reference.table.name == "likes")
				.unwrap()
				.clone();
			likes.strategy = MergeStrategy::Reassign;
			let error = merge::move_references(&db, &likes, absorbed.uuid(), survivor.uuid())
				.await
				.unwrap_err();
			assert!(matches!(error, DbError::Conflict { .. }), "{error:?}");

			// Policy-scoped connections can't move anyone's rows.
			let scoped = db.with_context(leptoskit_db::Context::new());
			let error = merge::count_references(&scoped, &likes, absorbed.uuid())
				.await
				.unwrap_err();
			assert!(matches!(error, DbError::Forbidden { .. }), "{error:?}");
		}
	})
	.await;
}

async fn futures_counts(db: &Db, data: &merge::AccountData, account: Id<Account>) -> Vec<u64> {
	let mut counts = Vec::new();

	for reference in &data.references {
		counts.push(
			merge::count_references(db, reference, account.uuid())
				.await
				.unwrap(),
		);
	}

	counts
}
