//! Mixins flattened into tables: their columns, rules, inserts and public
//! types, and the built-in timestamps, soft delete, and versioning, against
//! both databases.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use leptoskit_db::Db;
use leptoskit_db::HasMixin;
use leptoskit_db::Id;
use leptoskit_db::SoftDelete;
use leptoskit_db::Table;
use leptoskit_db::Timestamp;
use leptoskit_db::Timestamps;
use leptoskit_db::Versioned;
use leptoskit_db::mixin;
use leptoskit_db::table;
use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;

/// A postal address.
#[mixin(index(city))]
#[derive(Clone, PartialEq)]
pub struct Address {
	/// The street and number.
	#[db(max_len = 100)]
	pub street: String,
	pub city: String,
	/// For the courier only.
	#[db(private)]
	pub notes: Option<String>,
}

/// Who added a row, and when: the insert says who, the ORM says when.
#[mixin]
pub struct Stamp {
	pub by: Id<Account>,
	#[db(created_at)]
	pub at: Timestamp,
}

/// Someone who orders.
#[table(name = "accounts")]
pub struct Account {
	pub id: Id<Self>,
	pub name: String,
}

/// An order with two addresses.
#[table(name = "orders", public)]
pub struct Order {
	pub id: Id<Self>,
	#[db(min = 0)]
	pub total: i64,
	#[db(flatten, prefix = "billing_")]
	pub billing: Address,
	#[db(flatten, prefix = "shipping_")]
	pub shipping: Address,
	#[db(flatten)]
	pub times: Timestamps,
	#[db(flatten)]
	pub deleted: SoftDelete,
	#[db(flatten)]
	pub version: Versioned,
}

/// A note someone left on an account.
#[table(name = "account_notes", public)]
pub struct AccountNote {
	pub id: Id<Self>,
	pub body: String,
	#[db(flatten)]
	pub stamp: Stamp,
}

fn address(city: &str) -> Address {
	Address {
		street: "1 Marina".into(),
		city: city.into(),
		notes: Some("ring twice".into()),
	}
}

fn new_order(total: i64) -> NewOrder {
	NewOrder {
		total,
		billing: address("Lagos"),
		shipping: address("Abuja"),
	}
}

#[test]
fn mixin_columns_join_the_table_in_place() {
	assert_eq!(
		Order::COLUMNS,
		[
			"id",
			"total",
			"billing_street",
			"billing_city",
			"billing_notes",
			"shipping_street",
			"shipping_city",
			"shipping_notes",
			"created_at",
			"updated_at",
			"deleted_at",
			"version",
		]
	);
	assert_eq!(Order::CREATED_AT, Some("created_at"));
	assert_eq!(Order::UPDATED_AT, Some("updated_at"));
	assert_eq!(Order::DELETED_AT, Some("deleted_at"));
	assert_eq!(Order::VERSION, Some("version"));

	let table = Order::definition();
	let column = |name: &str| {
		table
			.columns
			.iter()
			.find(|column| column.name == name)
			.unwrap()
	};
	assert_eq!(
		column("billing_street").ty,
		ColumnType::Text { max_len: Some(100) }
	);
	assert_eq!(
		column("billing_street").comment.as_deref(),
		Some("The street and number.")
	);
	assert!(column("shipping_notes").nullable);

	// The mixin's own index follows it, once per prefix.
	let indexed: Vec<&[String]> = table
		.indexes
		.iter()
		.map(|index| index.columns.as_slice())
		.collect();
	assert!(indexed.contains(&["billing_city".to_string()].as_slice()));
	assert!(indexed.contains(&["shipping_city".to_string()].as_slice()));

	// A mixin's `Id<T>` column is a foreign key like any other.
	let notes = AccountNote::definition();
	assert_eq!(notes.foreign_keys.len(), 1);
	assert_eq!(notes.foreign_keys[0].columns, ["by"]);
	assert_eq!(notes.foreign_keys[0].references.name, "accounts");
}

#[test]
fn inserts_provide_only_what_the_orm_does_not() {
	let row = new_order(10).into_row();
	assert_eq!(row.times.created_at, row.times.updated_at);
	assert_eq!(row.deleted.deleted_at, None);
	assert_eq!(row.version.version, 0);

	// A mixin the insert partly provides has a `New…` struct.
	let account = Id::new();
	let note = NewAccountNote {
		body: "hello".into(),
		stamp: NewStamp { by: account },
	}
	.into_row();
	assert_eq!(note.stamp.by, account);

	let mut long = new_order(-1);
	long.billing.street = "x".repeat(101);
	let errors = long.validate().unwrap_err().to_string();
	assert_eq!(
		errors,
		"`total` can't be less than 0; `billing_street` can have at most 100 characters"
	);
}

#[test]
fn the_public_type_nests_each_mixin_as_its_public_type() {
	let public = PublicOrder::from(new_order(5).into_row());
	assert_eq!(public.billing.city, "Lagos");
	assert_eq!(public.version.version, 0);

	let json = serde_json::to_value(&public).unwrap();
	assert_eq!(json["shipping"]["city"], "Abuja");
	assert!(
		json["shipping"].get("notes").is_none(),
		"private fields stay on the server"
	);
	assert!(json.get("deleted").is_some());

	let mut invalid = public.clone();
	invalid.billing.street = "x".repeat(101);
	assert!(invalid.validate().is_err());

	let note = PublicAccountNote::from(
		NewAccountNote {
			body: "hi".into(),
			stamp: NewStamp { by: Id::new() },
		}
		.into_row(),
	);
	assert_eq!(note.stamp.by, note.stamp.by);
}

/// Generic over every table with timestamps.
fn created<T: HasMixin<Timestamps>>(row: &T) -> Timestamp {
	row.mixin().created_at
}

#[test]
fn generic_code_reaches_a_mixin_through_has_mixin() {
	let mut row = new_order(1).into_row();
	assert_eq!(created(&row), row.times.created_at);
	<Order as HasMixin<Timestamps>>::mixin_mut(&mut row).updated_at = row.times.created_at;
	assert_eq!(<Order as HasMixin<Versioned>>::mixin(&row).version, 0);
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

#[tokio::test(flavor = "multi_thread")]
async fn flattened_rows_round_trip_and_filter() {
	on_both(async |db| {
		let order = new_order(30).insert(&db).await.unwrap();
		new_order(40).insert(&db).await.unwrap();

		let found = Order::get(&db, order.id).await.unwrap();
		assert_eq!(found.billing, order.billing);
		assert_eq!(found.shipping.city, "Abuja");
		assert_eq!(found.times.created_at, order.times.created_at);

		let lagos = Order::query()
			.filter(|order| order.billing_city.eq("Lagos") & order.total.gt(35))
			.order_by(|order| order.created_at.desc())
			.all(&db)
			.await
			.unwrap();
		assert_eq!(lagos.len(), 1);

		let account = NewAccount { name: "Ada".into() }.insert(&db).await.unwrap();
		let note = NewAccountNote {
			body: "first".into(),
			stamp: NewStamp { by: account.id },
		}
		.insert(&db)
		.await
		.unwrap();
		assert_eq!(
			AccountNote::get(&db, note.id).await.unwrap().stamp.by,
			account.id
		);

		let orphan = NewAccountNote {
			body: "nobody".into(),
			stamp: NewStamp { by: Id::new() },
		}
		.insert(&db)
		.await
		.unwrap_err();
		assert!(
			matches!(orphan, leptoskit_db::DbError::ForeignKey { .. }),
			"{orphan:?}"
		);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn versioned_saves_refuse_stale_rows() {
	on_both(async |db| {
		let mut first = new_order(10).insert(&db).await.unwrap();
		let mut second = Order::get(&db, first.id).await.unwrap();

		first.total = 11;
		first.save(&db).await.unwrap();
		assert_eq!(first.version.version, 1);

		second.total = 12;
		let stale = second.save(&db).await.unwrap_err();
		assert!(stale.is_stale(), "{stale:?}");
		assert_eq!(Order::get(&db, first.id).await.unwrap().total, 11);

		// Reading again picks up the new version, and the save goes through.
		let mut fresh = Order::get(&db, first.id).await.unwrap();
		fresh.total = 12;
		fresh.save(&db).await.unwrap();
		assert_eq!(fresh.version.version, 2);

		// Bulk updates move the version too, so held rows go stale.
		let changed = Order::update()
			.filter(|order| order.id.eq(first.id))
			.set(|order| order.total, 13)
			.run(&db)
			.await
			.unwrap();
		assert_eq!(changed, 1);
		assert_eq!(Order::get(&db, first.id).await.unwrap().version.version, 3);
		assert!(fresh.save(&db).await.unwrap_err().is_stale());

		// A row that's gone is still not found, not stale.
		let gone = Order::get(&db, first.id).await.unwrap();
		Order::get(&db, first.id)
			.await
			.unwrap()
			.destroy(&db)
			.await
			.unwrap();
		let mut gone = gone;
		assert!(gone.save(&db).await.unwrap_err().is_not_found());
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_soft_delete_mixin_hides_rows() {
	on_both(async |db| {
		let mut order = new_order(10).insert(&db).await.unwrap();
		order.delete(&db).await.unwrap();
		assert!(order.deleted.deleted_at.is_some());
		assert!(Order::find(&db, order.id).await.unwrap().is_none());
		assert_eq!(Order::query().with_deleted().count(&db).await.unwrap(), 1);
	})
	.await;
}

/// An explicit runtime path takes another route through the mixin's macro;
/// the expansion must resolve its own names the same way.
#[table(name = "explicit_paths", crate = "leptoskit_db")]
pub struct ExplicitPath {
	pub id: Id<Self>,
	#[db(flatten)]
	pub times: Timestamps,
}

#[test]
fn an_explicit_runtime_path_flattens_too() {
	assert_eq!(ExplicitPath::COLUMNS, ["id", "created_at", "updated_at"]);
}
