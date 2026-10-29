//! Deletion policies: soft deletes that cascade along links, atomically and
//! through chains and cycles, and foreign keys checked when a transaction
//! commits, against both databases.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use leptoskit_db::Connection;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Id;
use leptoskit_db::SoftDelete;
use leptoskit_db::Table;
use leptoskit_db::Timestamp;
use leptoskit_db::Versioned;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;

/// A club.
#[table(name = "clubs")]
pub struct Club {
	pub id: Id<Self>,
	pub name: String,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// Someone in a club: gone when the club goes.
#[table(name = "members")]
pub struct Member {
	pub id: Id<Self>,
	#[db(on_soft_delete = cascade, backlink = members)]
	pub club_id: Id<Club>,
	pub name: String,
	#[db(flatten)]
	pub deleted: SoftDelete,
	#[db(flatten)]
	pub version: Versioned,
}

/// What a member posted: gone when the member goes.
#[table(name = "notices")]
pub struct Notice {
	pub id: Id<Self>,
	#[db(on_soft_delete = cascade)]
	pub member_id: Id<Member>,
	pub text: String,
	#[db(updated_at)]
	pub updated_at: Timestamp,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// A member's badge: it stays when the member goes, since its link doesn't
/// cascade.
#[table(name = "badges")]
pub struct Badge {
	pub id: Id<Self>,
	pub member_id: Id<Member>,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// A folder in a tree, which goes with its parent.
#[table(name = "folders")]
pub struct Folder {
	pub id: Id<Self>,
	#[db(on_soft_delete = cascade)]
	pub parent_id: Option<Id<Folder>>,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// A seat that must point at a table by the time its transaction commits.
#[table(name = "seats")]
pub struct Seat {
	pub id: Id<Self>,
	#[db(on_delete = deferred_restrict)]
	pub booth_id: Id<Booth>,
}

/// Somewhere seats point at, with a key chosen up front.
#[table(name = "booths")]
pub struct Booth {
	#[db(key, generate = false)]
	pub id: Id<Self>,
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

/// A club with two members, each with a notice and a badge.
async fn club(db: &Db, name: &str) -> (Club, Vec<Member>) {
	let club = NewClub { name: name.into() }.insert(db).await.unwrap();
	let mut members = Vec::new();

	for person in ["Ada", "Grace"] {
		let member = NewMember {
			club_id: club.id,
			name: person.into(),
		}
		.insert(db)
		.await
		.unwrap();
		NewNotice {
			member_id: member.id,
			text: format!("{person} was here"),
		}
		.insert(db)
		.await
		.unwrap();
		NewBadge {
			member_id: member.id,
		}
		.insert(db)
		.await
		.unwrap();
		members.push(member);
	}

	(club, members)
}

async fn alive<T: Table>(db: &Db) -> u64 {
	leptoskit_db::Query::<T>::new().count(db).await.unwrap()
}

#[test]
fn deferred_keys_are_checked_at_commit() {
	let table = Seat::definition();
	assert!(table.foreign_keys[0].deferred);
	assert_eq!(
		table.foreign_keys[0].on_delete,
		leptoskit_db_schema::ReferentialAction::NoAction
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn soft_deletes_cascade_along_links() {
	on_both(async |db| {
		let (mut doomed, members) = club(&db, "Doomed").await;
		club(&db, "Kept").await;
		let held = Member::get(&db, members[0].id).await.unwrap();

		doomed.delete(&db).await.unwrap();

		// The club's members and their notices went with it; the other
		// club's didn't, and badges don't cascade.
		assert_eq!(doomed.members().count(&db).await.unwrap(), 0);
		assert_eq!(alive::<Member>(&db).await, 2);
		assert_eq!(alive::<Notice>(&db).await, 2);
		assert_eq!(alive::<Badge>(&db).await, 4);

		let gone = Member::query()
			.with_deleted()
			.filter(|member| member.club_id.eq(doomed.id))
			.all(&db)
			.await
			.unwrap();
		assert_eq!(gone.len(), 2);
		assert!(
			gone.iter()
				.all(|member| member.deleted.deleted_at == doomed.deleted_at)
		);
		// A cascade is a write, so versioned rows move on.
		assert!(gone.iter().all(|member| member.version.version == 1));
		let mut held = held;
		held.name = "stale".into();
		assert!(held.save(&db).await.unwrap_err().is_stale());

		let notice = Notice::query()
			.with_deleted()
			.filter(|notice| notice.member_id.eq(members[1].id))
			.one(&db)
			.await
			.unwrap();
		assert_eq!(notice.deleted_at, doomed.deleted_at);
		assert_eq!(Some(notice.updated_at), doomed.deleted_at);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bulk_soft_deletes_cascade_too() {
	on_both(async |db| {
		club(&db, "One").await;
		club(&db, "Two").await;
		club(&db, "Kept").await;

		let deleted = Club::query()
			.filter(|club| club.name.ne("Kept"))
			.delete(&db)
			.await
			.unwrap();
		assert_eq!(
			deleted, 2,
			"the count is of clubs, not of everything it reached"
		);
		assert_eq!(alive::<Member>(&db).await, 2);
		assert_eq!(alive::<Notice>(&db).await, 2);

		// Deleting again finds nothing new to cascade.
		assert_eq!(
			Club::query()
				.filter(|club| club.name.ne("Kept"))
				.delete(&db)
				.await
				.unwrap(),
			0
		);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cascade_joins_the_callers_transaction() {
	on_both(async |db| {
		let (club, _) = club(&db, "Undone").await;

		let outcome = db
			.transaction(async |tx| {
				let mut club = Club::get(tx, club.id).await?;
				club.delete(tx).await?;
				assert_eq!(alive_in(tx).await, 0, "the transaction sees its cascade");
				Err::<(), _>(DbError::NotFound {
					table: "abandoned".into(),
				})
			})
			.await;
		assert!(outcome.is_err());

		// Rolled back together: the club and its members are all still here.
		assert!(Club::find(&db, club.id).await.unwrap().is_some());
		assert_eq!(alive::<Member>(&db).await, 2);
	})
	.await;
}

async fn alive_in(connection: &impl Connection) -> u64 {
	Member::query().count(connection).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn cascades_follow_trees_and_stop_at_cycles() {
	on_both(async |db| {
		let mut root = NewFolder { parent_id: None }.insert(&db).await.unwrap();
		let child = NewFolder {
			parent_id: Some(root.id),
		}
		.insert(&db)
		.await
		.unwrap();
		let grandchild = NewFolder {
			parent_id: Some(child.id),
		}
		.insert(&db)
		.await
		.unwrap();

		// Close the loop: the root's parent is its own grandchild.
		root.parent_id = Some(grandchild.id);
		root.save(&db).await.unwrap();

		root.delete(&db).await.unwrap();
		assert_eq!(alive::<Folder>(&db).await, 0);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn deferred_keys_let_a_transaction_insert_in_any_order() {
	on_both(async |db| {
		let booth = Id::<Booth>::new();
		let seated = db
			.transaction(async |tx| {
				// The booth doesn't exist yet; the key waits for the commit.
				NewSeat { booth_id: booth }.insert(tx).await?;
				NewBooth { id: booth }.insert(tx).await?;
				Ok::<_, DbError>(())
			})
			.await;
		seated.unwrap();
		assert_eq!(Seat::by_booth(booth).count(&db).await.unwrap(), 1);

		// Still checked, just later: a seat pointing nowhere fails to commit.
		let orphan = db
			.transaction(async |tx| {
				NewSeat {
					booth_id: Id::new(),
				}
				.insert(tx)
				.await?;
				Ok::<_, DbError>(())
			})
			.await;
		assert!(orphan.is_err());
		assert_eq!(alive::<Seat>(&db).await, 1);
	})
	.await;
}
