//! Hooks, `created_by`/`updated_by`, and slugs against both databases:
//! every hook runs inside its write's transaction, so a failing hook rolls
//! the write back, and bulk writes refuse to skip hooks by accident.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use leptoskit_db::Actor;
use leptoskit_db::AfterDelete;
use leptoskit_db::AfterInsert;
use leptoskit_db::AfterUpdate;
use leptoskit_db::BeforeWrite;
use leptoskit_db::Connection;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Slug;
use leptoskit_db::Timestamp;
use leptoskit_db::WriteContext;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;

/// Someone who writes. Writes record who made them.
#[table(name = "writers")]
pub struct Writer {
	pub id: Id<Self>,
	pub name: String,
}

/// An order whose total the `before_write` hook keeps, and whose status
/// changes the `after_update` hook records as events.
#[table(
	name = "orders",
	hooks(before_write, after_insert, after_update, after_delete)
)]
pub struct Order {
	pub id: Id<Self>,
	#[db(unique)]
	pub reference: String,
	pub status: String,
	pub quantity: i64,
	pub unit_price: i64,
	pub total: i64,
	#[db(created_by)]
	pub created_by: Option<Id<Writer>>,
	#[db(updated_by)]
	pub updated_by: Option<Id<Writer>>,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// What happened to an order, written by its hooks.
#[table(name = "order_events")]
pub struct OrderEvent {
	pub id: Id<Self>,
	/// Events outlive the orders they're about.
	#[db(no_foreign_key)]
	pub order_id: Id<Order>,
	pub kind: String,
	#[db(created_by)]
	pub created_by: Option<Id<Writer>>,
}

impl BeforeWrite for Order {
	fn before_write(&mut self, cx: &WriteContext<'_, Self>) -> leptoskit_db::Result<()> {
		if self.quantity < 0 {
			return Err(DbError::Forbidden {
				table: "orders".into(),
				message: "quantities can't be negative".into(),
			});
		}

		match self.status.as_str() {
			// What picks the row to write can't change.
			"rekey" => self.id = Id::new(),
			"rename" => self.reference = "renamed".into(),
			_ => {}
		}

		if self.status == "forge" {
			// A hook can't write who made the change; the layer fills it in
			// again after the hook.
			self.created_by = Some(Id::new());
			self.updated_by = Some(Id::new());
		}

		if let Some(old) = cx.old()
			&& old.status == "shipped"
			&& self.status != "shipped"
		{
			return Err(DbError::Forbidden {
				table: "orders".into(),
				message: "shipped orders stay shipped".into(),
			});
		}

		self.total = self.quantity * self.unit_price;
		Ok(())
	}
}

impl AfterInsert for Order {
	async fn after_insert(&self, tx: &impl Connection) -> leptoskit_db::Result<()> {
		event(tx, self.id, "placed").await
	}
}

impl AfterUpdate for Order {
	async fn after_update(
		old: &Self,
		new: &Self,
		tx: &impl Connection,
	) -> leptoskit_db::Result<()> {
		if old.status == new.status {
			return Ok(());
		}

		if new.status == "explode" {
			return Err(DbError::Forbidden {
				table: "orders".into(),
				message: "the hook refused".into(),
			});
		}

		event(tx, new.id, &format!("{} -> {}", old.status, new.status)).await
	}
}

impl AfterDelete for Order {
	async fn after_delete(&self, tx: &impl Connection) -> leptoskit_db::Result<()> {
		let kind = if self.deleted_at.is_some() {
			"archived"
		} else {
			"removed"
		};
		event(tx, self.id, kind).await
	}
}

async fn event(tx: &impl Connection, order_id: Id<Order>, kind: &str) -> leptoskit_db::Result<()> {
	NewOrderEvent {
		order_id,
		kind: kind.into(),
	}
	.insert(tx)
	.await
	.map(|_| ())
}

/// A note on an order, soft-deleted with it, recording who did.
#[table(name = "order_notes")]
pub struct OrderNote {
	pub id: Id<Self>,
	#[db(on_soft_delete = cascade)]
	pub order_id: Id<Order>,
	#[db(updated_by)]
	pub updated_by: Option<Id<Writer>>,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// A row that needs someone acting to be written at all.
#[table(name = "signed_notes")]
pub struct SignedNote {
	pub id: Id<Self>,
	pub body: String,
	#[db(created_by)]
	pub author: Id<Writer>,
	#[db(updated_by)]
	pub editor: uuid::Uuid,
}

/// An article with a unique slug made from its title, freed when an article
/// is soft-deleted.
#[table(name = "articles")]
pub struct Article {
	pub id: Id<Self>,
	#[db(unique)]
	pub code: String,
	pub title: String,
	#[db(slug(from = title), unique(except = deleted_at))]
	pub slug: Slug,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// A tag, linked to orders.
#[table(name = "labels")]
pub struct Label {
	pub id: Id<Self>,
	pub name: String,
}

/// A label on an order. Removing one records it.
#[leptoskit_db::link(name = "order_labels", from = Order as labels, to = Label as orders, hooks(after_delete))]
pub struct OrderLabel {
	pub note: String,
}

impl AfterDelete for OrderLabel {
	async fn after_delete(&self, tx: &impl Connection) -> leptoskit_db::Result<()> {
		event(tx, self.order_id, &format!("unlabelled {}", self.note)).await
	}
}

/// A row anyone may insert but only some may change.
#[table(name = "guarded", hooks(after_insert))]
#[policy(select, insert, |_, _| Expr::always())]
#[policy(update, delete, |row, _| row.open.eq(true))]
pub struct Guarded {
	pub id: Id<Self>,
	pub open: bool,
	pub touched: bool,
}

impl AfterInsert for Guarded {
	async fn after_insert(&self, tx: &impl Connection) -> leptoskit_db::Result<()> {
		// The hook's write obeys the request's policies too.
		let mut copy = Guarded::get(tx, self.id).await?;
		copy.touched = true;
		copy.save(tx).await
	}
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

async fn writer(db: &Db, name: &str) -> (Id<Writer>, Db) {
	let id = NewWriter { name: name.into() }
		.insert(&db.system("seeding writers"))
		.await
		.unwrap()
		.id;
	(id, db.with_context(Context::new().with(Actor::of(id))))
}

async fn events(db: &Db, order: Id<Order>) -> Vec<String> {
	let mut events = OrderEvent::query()
		.filter(|event| event.order_id.eq(order))
		.all(db)
		.await
		.unwrap();
	events.sort_by_key(|event| event.id);
	events.into_iter().map(|event| event.kind).collect()
}

fn new_order(quantity: i64) -> NewOrder {
	NewOrder {
		reference: Id::<Order>::new().to_string(),
		status: "open".into(),
		quantity,
		unit_price: 3,
		total: 0,
	}
}

#[tokio::test]
async fn hooks_run_inside_every_single_row_write() {
	on_both(async |db| {
		let (ada, as_ada) = writer(&db, "Ada").await;
		let (grace, as_grace) = writer(&db, "Grace").await;

		// Insert: `before_write` fills the total, `after_insert` writes an
		// event, and both record who acted.
		let mut order = new_order(4).insert(&as_ada).await.unwrap();
		assert_eq!(order.total, 12);
		assert_eq!(order.created_by, Some(ada));
		assert_eq!(order.updated_by, Some(ada));
		assert_eq!(events(&db, order.id).await, ["placed"]);
		let placed = OrderEvent::query().one(&db).await.unwrap();
		assert_eq!(placed.created_by, Some(ada));

		// Save: `before_write` sees the old row, `after_update` sees both.
		order.quantity = 5;
		order.status = "paid".into();
		order.save(&as_grace).await.unwrap();
		assert_eq!(order.total, 15);
		assert_eq!(order.created_by, Some(ada));
		assert_eq!(order.updated_by, Some(grace));
		assert_eq!(events(&db, order.id).await, ["placed", "open -> paid"]);

		// A failing `before_write` writes nothing.
		order.quantity = -1;
		let refused = order.save(&as_grace).await.unwrap_err();
		assert_eq!(refused.to_string(), "quantities can't be negative");
		let stored = Order::get(&db, order.id).await.unwrap();
		assert_eq!(stored.quantity, 5);

		// A failing `after_update` rolls back the change and its events.
		let mut stored = stored;
		stored.status = "explode".into();
		assert!(stored.save(&as_ada).await.unwrap_err().is_forbidden());
		let mut stored = Order::get(&db, order.id).await.unwrap();
		assert_eq!(stored.status, "paid");
		assert_eq!(events(&db, order.id).await.len(), 2);

		// The old row the hook sees is the stored one, not the caller's.
		stored.status = "shipped".into();
		stored.save(&as_ada).await.unwrap();
		let mut stale = stored;
		stale.status = "open".into();
		assert_eq!(
			stale.save(&as_ada).await.unwrap_err().to_string(),
			"shipped orders stay shipped"
		);

		// A hook can't change the key that picks the row.
		let mut rekeyed = new_order(1).insert(&as_ada).await.unwrap();
		rekeyed.status = "rekey".into();
		let refused = rekeyed.save(&as_ada).await.unwrap_err();
		assert!(matches!(refused, DbError::ReadOnly { .. }), "{refused}");
		assert!(refused.to_string().contains("changed `id`"), "{refused}");

		// A hook can't forge who wrote the row.
		let mut forged = new_order(1);
		forged.status = "forge".into();
		let forged = forged.insert(&as_grace).await.unwrap();
		assert_eq!(forged.created_by, Some(grace));
		assert_eq!(forged.updated_by, Some(grace));

		// Soft delete and destroy run `after_delete` with the row as the
		// delete left it.
		let mut archived = new_order(1).insert(&as_ada).await.unwrap();
		archived.delete(&as_grace).await.unwrap();
		assert!(archived.deleted_at.is_some());
		assert_eq!(archived.updated_by, Some(grace));
		assert_eq!(events(&db, archived.id).await, ["placed", "archived"]);
		let removed = new_order(1).insert(&as_ada).await.unwrap();
		let removed_id = removed.id;
		removed.destroy(&as_ada).await.unwrap();
		assert_eq!(events(&db, removed_id).await, ["placed", "removed"]);
		assert!(
			Order::query()
				.with_deleted()
				.filter(|o| o.id.eq(removed_id))
				.first(&db)
				.await
				.unwrap()
				.is_none()
		);

		// Deleting a row that isn't there runs nothing.
		let mut gone = Order::query()
			.with_deleted()
			.filter(|o| o.id.eq(archived.id))
			.one(&db)
			.await
			.unwrap();
		gone.id = Id::new();
		assert!(gone.delete(&as_ada).await.unwrap_err().is_not_found());
		assert!(
			Order::query()
				.with_deleted()
				.filter(|o| o.id.eq(archived.id))
				.one(&db)
				.await
				.unwrap()
				.destroy(&as_ada)
				.await
				.is_ok()
		);
	})
	.await;
}

#[tokio::test]
async fn upserts_run_the_hooks_of_what_they_do() {
	on_both(async |db| {
		let (ada, as_ada) = writer(&db, "Ada").await;
		let (grace, as_grace) = writer(&db, "Grace").await;
		let order = new_order(2).insert(&as_ada).await.unwrap();

		// Overwriting an existing row is an update: `before_write` sees the
		// old row, and `created_by` stays.
		let replacement = NewOrder {
			reference: order.reference.clone(),
			status: "paid".into(),
			quantity: 7,
			unit_price: 2,
			total: 0,
		};
		let stored = replacement
			.upsert(&as_grace, |o| o.reference)
			.await
			.unwrap();
		assert_eq!(stored.id, order.id);
		assert_eq!(stored.total, 14);
		assert_eq!(stored.created_by, Some(ada));
		assert_eq!(stored.updated_by, Some(grace));
		assert_eq!(events(&db, order.id).await, ["placed", "open -> paid"]);

		// Upserting a new row is an insert.
		let fresh = new_order(1)
			.upsert(&as_grace, |o| o.reference)
			.await
			.unwrap();
		assert_eq!(fresh.created_by, Some(grace));
		assert_eq!(events(&db, fresh.id).await, ["placed"]);

		// The update's `before_write` can refuse, and changes nothing.
		let mut shipped = Order::get(&db, order.id).await.unwrap();
		shipped.status = "shipped".into();
		shipped.save(&as_ada).await.unwrap();
		let mut reopen = new_order(1);
		reopen.reference = order.reference.clone();
		assert_eq!(
			reopen
				.upsert(&as_ada, |o| o.reference)
				.await
				.unwrap_err()
				.to_string(),
			"shipped orders stay shipped"
		);

		// A hook that changes what picks the row is refused, on either path.
		for reference in [fresh.reference.clone(), "brand-new".to_string()] {
			let mut renamed = new_order(1);
			renamed.reference = reference;
			renamed.status = "rename".into();
			let refused = renamed.upsert(&as_ada, |o| o.reference).await.unwrap_err();
			assert!(
				refused.to_string().contains("changed `reference`"),
				"{refused}"
			);
		}

		assert_eq!(Order::query().count(&db).await.unwrap(), 2);
	})
	.await;
}

#[tokio::test]
async fn hooks_join_the_callers_transaction() {
	on_both(async |db| {
		let (_, as_ada) = writer(&db, "Ada").await;
		// An in-memory SQLite database has one connection, which the
		// transaction holds, so reads inside it go through it.
		let tx = as_ada.begin().await.unwrap();
		let order = new_order(1).insert(&tx).await.unwrap();
		assert_eq!(OrderEvent::query().count(&tx).await.unwrap(), 1);
		tx.rollback().await.unwrap();
		assert!(Order::find(&db, order.id).await.unwrap().is_none());
		assert_eq!(OrderEvent::query().count(&db).await.unwrap(), 0);

		let order = as_ada
			.transaction(async |tx| new_order(1).insert(tx).await)
			.await
			.unwrap();
		assert_eq!(events(&db, order.id).await, ["placed"]);
	})
	.await;
}

#[tokio::test]
async fn bulk_writes_refuse_to_skip_hooks_unless_told() {
	on_both(async |db| {
		let (ada, as_ada) = writer(&db, "Ada").await;
		let (grace, as_grace) = writer(&db, "Grace").await;

		for quantity in 1..=3 {
			new_order(quantity).insert(&as_ada).await.unwrap();
		}

		let refused = Order::update()
			.set(|o| o.status, "held".to_string())
			.run(&as_ada)
			.await
			.unwrap_err();
		assert!(matches!(refused, DbError::HooksSkipped { ref table } if table == "orders"));
		assert!(matches!(
			Order::query().delete(&as_ada).await.unwrap_err(),
			DbError::HooksSkipped { .. }
		));
		assert!(matches!(
			Order::query().destroy(&as_ada).await.unwrap_err(),
			DbError::HooksSkipped { .. }
		));
		assert_eq!(
			Order::query()
				.filter(|o| o.status.eq("held"))
				.count(&db)
				.await
				.unwrap(),
			0
		);

		// Told to, a bulk update skips the hooks but still records who acted.
		let changed = Order::update()
			.filter(|o| o.quantity.gt(1))
			.set(|o| o.status, "held".to_string())
			.allow_skipping_hooks()
			.run(&as_grace)
			.await
			.unwrap();
		assert_eq!(changed, 2);
		let held = Order::query()
			.filter(|o| o.status.eq("held"))
			.all(&db)
			.await
			.unwrap();
		assert!(
			held.iter()
				.all(|o| o.updated_by == Some(grace) && o.created_by == Some(ada))
		);
		assert_eq!(OrderEvent::query().count(&db).await.unwrap(), 3);

		assert_eq!(
			Order::query()
				.allow_skipping_hooks()
				.delete(&as_grace)
				.await
				.unwrap(),
			3
		);
		let archived = Order::query().with_deleted().all(&db).await.unwrap();
		assert!(archived.iter().all(|o| o.updated_by == Some(grace)));
		assert_eq!(
			Order::query()
				.with_deleted()
				.allow_skipping_hooks()
				.destroy(&as_grace)
				.await
				.unwrap(),
			3
		);
		assert_eq!(OrderEvent::query().count(&db).await.unwrap(), 3);

		// Who wrote a row isn't something a bulk write may set.
		let refused = OrderEvent::update()
			.set(|e| e.created_by, Some(grace))
			.run(&db)
			.await
			.unwrap_err();
		assert!(matches!(refused, DbError::ReadOnly { .. }), "{refused}");
		assert!(
			refused.to_string().contains("records who wrote"),
			"{refused}"
		);
	})
	.await;
}

#[tokio::test]
async fn required_actor_columns_need_someone_acting() {
	on_both(async |db| {
		let (ada, as_ada) = writer(&db, "Ada").await;
		let (grace, as_grace) = writer(&db, "Grace").await;

		let refused = NewSignedNote { body: "hi".into() }.insert(&db).await.unwrap_err();
		assert!(matches!(&refused, DbError::NoActor { table, column } if table == "signed_notes" && column == "author"));
		assert_eq!(refused.to_string(), "a `signed_notes` row records who wrote it in `author`, and nobody is acting");
		assert!(NewSignedNote { body: "hi".into() }.insert(&db.system("no request")).await.unwrap_err().to_string().contains("nobody is acting"));

		let mut note = NewSignedNote { body: "hi".into() }.insert(&as_ada).await.unwrap();
		assert_eq!((note.author, note.editor), (ada, ada.uuid()));

		// A bypass a request makes still records who asked.
		let as_grace_system = as_grace.system("support fixed a typo");
		assert_eq!(as_grace_system.actor(), Some(Actor::of(grace)));
		note.body = "hello".into();
		note.save(&as_grace_system).await.unwrap();
		assert_eq!((note.author, note.editor), (ada, grace.uuid()));
		let stored = SignedNote::get(&db, note.id).await.unwrap();
		assert_eq!((stored.author, stored.editor), (ada, grace.uuid()));

		// Saving with nobody acting is refused; so are bulk updates.
		note.body = "anonymous".into();
		assert!(matches!(note.save(&db).await.unwrap_err(), DbError::NoActor { ref column, .. } if column == "editor"));
		assert!(matches!(
			SignedNote::update().set(|n| n.body, "x".to_string()).run(&db).await.unwrap_err(),
			DbError::NoActor { .. }
		));
		assert_eq!(SignedNote::update().set(|n| n.body, "x".to_string()).run(&as_ada).await.unwrap(), 1);
		assert_eq!(SignedNote::get(&db, note.id).await.unwrap().editor, ada.uuid());
	})
	.await;
}

#[tokio::test]
async fn slugs_are_unique_and_kept() {
	on_both(async |db| {
		let article = |code: &str, title: &str| {
			NewArticle {
				code: code.into(),
				title: title.into(),
			}
		};

		let first = article("a", "Hello, World!").insert(&db).await.unwrap();
		assert_eq!(first.slug.as_inner(), "hello-world");
		let second = article("b", "hello world").insert(&db).await.unwrap();
		assert_eq!(second.slug.as_inner(), "hello-world-2");
		let mut third = article("c", "Hello World").insert(&db).await.unwrap();
		assert_eq!(third.slug.as_inner(), "hello-world-3");
		let other = article("d", "日本").insert(&db).await.unwrap();
		assert_eq!(other.slug.as_inner(), "untitled");
		// A longer slug that starts the same isn't taken by them.
		let longer = article("e", "Hello World Again").insert(&db).await.unwrap();
		assert_eq!(longer.slug.as_inner(), "hello-world-again");

		// Saving keeps the slug, even when the title changes.
		third.title = "Something else".into();
		third.save(&db).await.unwrap();
		assert_eq!(
			Article::get(&db, third.id).await.unwrap().slug.as_inner(),
			"hello-world-3"
		);

		// A soft-deleted article frees its slug.
		let mut first = first;
		first.delete(&db).await.unwrap();
		let again = article("f", "Hello world").insert(&db).await.unwrap();
		assert_eq!(again.slug.as_inner(), "hello-world");

		// Upserting an existing row keeps its slug; a new one gets its own.
		let upserted = article("b", "Hello World")
			.upsert(&db, |a| a.code)
			.await
			.unwrap();
		assert_eq!(upserted.id, second.id);
		assert_eq!(upserted.slug.as_inner(), "hello-world-2");
		let new = article("g", "Hello World")
			.upsert(&db, |a| a.code)
			.await
			.unwrap();
		assert_eq!(new.slug.as_inner(), "hello-world-4");
	})
	.await;
}

#[tokio::test]
async fn removing_a_link_runs_its_hooks() {
	on_both(async |db| {
		let (_, as_ada) = writer(&db, "Ada").await;
		let order = new_order(1).insert(&as_ada).await.unwrap();
		let label = NewLabel {
			name: "gift".into(),
		}
		.insert(&db)
		.await
		.unwrap();
		order
			.labels()
			.add(
				&as_ada,
				label.id,
				OrderLabelProperties {
					note: "wrap it".into(),
				},
			)
			.await
			.unwrap();

		assert!(order.labels().remove(&as_ada, label.id).await.unwrap());
		assert!(!order.labels().remove(&as_ada, label.id).await.unwrap());
		assert_eq!(
			events(&db, order.id).await,
			["placed", "unlabelled wrap it"]
		);
		assert_eq!(order.labels().count(&db).await.unwrap(), 0);
	})
	.await;
}

#[tokio::test]
async fn hook_writes_obey_the_requests_policies() {
	on_both(async |db| {
		// The hook changes the row, which needs it to be open: a closed
		// row's hook is refused, and the insert rolls back with it.
		let closed = NewGuarded {
			open: false,
			touched: false,
		}
		.insert(&db)
		.await;
		assert!(closed.unwrap_err().is_forbidden());
		assert_eq!(
			Guarded::query().count(&db.system("check")).await.unwrap(),
			0
		);

		let opened = NewGuarded {
			open: true,
			touched: false,
		}
		.insert(&db)
		.await
		.unwrap();
		assert!(Guarded::get(&db, opened.id).await.unwrap().touched);

		// A bypass skips the policies in its hooks too.
		let system = db.system("seeding");
		let row = NewGuarded {
			open: false,
			touched: false,
		}
		.insert(&system)
		.await
		.unwrap();
		assert!(Guarded::get(&system, row.id).await.unwrap().touched);
	})
	.await;
}

#[tokio::test]
async fn soft_delete_cascades_record_who_acted() {
	on_both(async |db| {
		let (ada, as_ada) = writer(&db, "Ada").await;
		let (grace, as_grace) = writer(&db, "Grace").await;
		let mut order = new_order(1).insert(&as_ada).await.unwrap();
		let note = NewOrderNote { order_id: order.id }
			.insert(&as_ada)
			.await
			.unwrap();
		assert_eq!(note.updated_by, Some(ada));

		order.delete(&as_grace).await.unwrap();
		let note = OrderNote::query()
			.with_deleted()
			.filter(|n| n.id.eq(note.id))
			.one(&db)
			.await
			.unwrap();
		assert!(note.deleted_at.is_some());
		assert_eq!(note.updated_by, Some(grace));

		// Nobody acting leaves an optional column empty.
		let mut other = new_order(1).insert(&as_ada).await.unwrap();
		let note = NewOrderNote { order_id: other.id }
			.insert(&as_ada)
			.await
			.unwrap();
		other.delete(&db.system("cleanup")).await.unwrap();
		let note = OrderNote::query()
			.with_deleted()
			.filter(|n| n.id.eq(note.id))
			.one(&db)
			.await
			.unwrap();
		assert_eq!(note.updated_by, None);
	})
	.await;
}
