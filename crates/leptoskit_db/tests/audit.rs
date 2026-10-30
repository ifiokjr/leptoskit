//! The audit log against both databases: every write the database layer
//! makes to an audited table is recorded in its transaction, with who made
//! it and the row before and after, never its `secret` columns, and reading
//! the log is refused unless the request may.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use std::collections::HashMap;

use leptoskit_db::Actor;
use leptoskit_db::AuditEntry;
use leptoskit_db::AuditOp;
use leptoskit_db::AuditReader;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest::test_runner::TestCaseError;
use proptest::test_runner::TestRunner;
use uuid::Uuid;

/// A wallet: its owner may change it, and its PIN is never recorded.
#[table(name = "wallets", audit)]
#[policy(select, |_, _| Expr::always())]
#[policy(insert, update, delete, |wallet, cx| match cx.actor() {
	Some(actor) => wallet.owner.eq(actor.uuid()),
	None => Expr::never(),
})]
pub struct Wallet {
	pub id: Id<Self>,
	pub owner: Uuid,
	#[db(unique)]
	pub name: String,
	pub balance: i64,
	#[db(private)]
	pub email: String,
	#[db(secret)]
	pub pin: String,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// A payment from a wallet, soft-deleted with it.
#[table(name = "payments", namespace = "billing", audit)]
pub struct Payment {
	pub id: Id<Self>,
	#[db(on_soft_delete = cascade)]
	pub wallet_id: Id<Wallet>,
	pub amount: i64,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// Not audited: nothing about it is recorded.
#[table(name = "scratch")]
pub struct Scratch {
	pub id: Id<Self>,
	pub note: String,
}

async fn create_schema(db: &Db) {
	let dialect = db.dialect();
	let schema = leptoskit_db::schema(dialect).unwrap();
	assert!(
		schema
			.tables
			.iter()
			.any(|table| table.name.name == "_leptoskit_audit"),
		"an audited table brings the audit log into the schema"
	);
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

fn acting(db: &Db, actor: Uuid) -> Db {
	db.with_context(Context::new().with(Actor::from_uuid(actor)))
}

fn wallet(owner: Uuid, name: &str) -> NewWallet {
	NewWallet {
		owner,
		name: name.into(),
		balance: 0,
		email: format!("{name}@example.com"),
		pin: format!("pin-of-{name}"),
	}
}

async fn log(db: &Db) -> Vec<AuditEntry> {
	AuditEntry::query()
		.order_by(|entry| entry.id.asc())
		.all(&db.system("reading the audit log"))
		.await
		.unwrap()
}

fn field<'a>(
	row: Option<&'a leptoskit_db::Json<serde_json::Value>>,
	name: &str,
) -> Option<&'a serde_json::Value> {
	row.and_then(|row| row.0.get(name))
}

#[tokio::test]
async fn single_row_writes_are_recorded_with_who_and_what() {
	on_both(async |db| {
		let ada = Uuid::now_v7();
		let as_ada = acting(&db, ada);

		let mut stored = wallet(ada, "ada").insert(&as_ada).await.unwrap();
		stored.balance = 10;
		stored.save(&as_ada).await.unwrap();
		let upserted = NewWallet {
			balance: 20,
			..wallet(ada, "ada")
		}
		.upsert(&as_ada, |w| w.name)
		.await
		.unwrap();
		assert_eq!(upserted.id, stored.id);
		let mut stored = upserted;
		stored
			.delete(&as_ada.system("closing the account"))
			.await
			.unwrap();
		Wallet::query()
			.with_deleted()
			.one(&db)
			.await
			.unwrap()
			.destroy(&as_ada)
			.await
			.unwrap();
		NewScratch {
			note: "not audited".into(),
		}
		.insert(&db)
		.await
		.unwrap();

		let entries = log(&db).await;
		let ops: Vec<AuditOp> = entries.iter().map(|entry| entry.op).collect();
		assert_eq!(
			ops,
			[
				AuditOp::Insert,
				AuditOp::Update,
				AuditOp::Update,
				AuditOp::Delete,
				AuditOp::Delete
			]
		);
		assert!(entries.iter().all(|entry| entry.table_name == "wallets"));
		assert!(
			entries
				.iter()
				.all(|entry| entry.row_key == format!("[\"{}\"]", stored.id))
		);
		assert!(
			entries
				.iter()
				.all(|entry| entry.actor() == Some(Actor::from_uuid(ada)))
		);
		let reasons: Vec<Option<&str>> = entries
			.iter()
			.map(|entry| entry.reason.as_deref())
			.collect();
		assert_eq!(
			reasons,
			[None, None, None, Some("closing the account"), None]
		);

		// Before and after, as stored.
		assert!(entries[0].before.is_none());
		assert_eq!(
			field(entries[0].after.as_ref(), "balance"),
			Some(&serde_json::json!(0))
		);
		assert_eq!(
			field(entries[1].before.as_ref(), "balance"),
			Some(&serde_json::json!(0))
		);
		assert_eq!(
			field(entries[1].after.as_ref(), "balance"),
			Some(&serde_json::json!(10))
		);
		assert_eq!(
			field(entries[2].after.as_ref(), "balance"),
			Some(&serde_json::json!(20))
		);
		assert_eq!(
			field(entries[3].before.as_ref(), "deleted_at"),
			Some(&serde_json::Value::Null)
		);
		assert!(
			field(entries[3].after.as_ref(), "deleted_at")
				.is_some_and(serde_json::Value::is_string)
		);
		assert!(entries[4].after.is_none());
		assert_eq!(
			field(entries[4].before.as_ref(), "email"),
			Some(&serde_json::json!("ada@example.com"))
		);

		// The secret column is never recorded, in any form.
		for entry in &entries {
			for row in [&entry.before, &entry.after].into_iter().flatten() {
				assert!(row.0.get("pin").is_none(), "{:?}", row.0);
				assert!(!row.0.to_string().contains("pin-of-ada"), "{:?}", row.0);
			}
		}

		// `for_row` finds one row's history.
		let history = AuditEntry::for_row(&stored)
			.unwrap()
			.all(&db.system("history"))
			.await
			.unwrap();
		assert_eq!(history.len(), 5);
		assert_eq!(
			AuditEntry::for_table::<Scratch>()
				.count(&db.system("x"))
				.await
				.unwrap(),
			0
		);
	})
	.await;
}

#[tokio::test]
async fn refused_and_rolled_back_writes_record_nothing() {
	on_both(async |db| {
		let ada = Uuid::now_v7();
		let grace = Uuid::now_v7();
		let mut stored = wallet(ada, "ada").insert(&acting(&db, ada)).await.unwrap();
		assert_eq!(log(&db).await.len(), 1);

		// Grace may not change Ada's wallet, nor insert one in her name.
		stored.balance = 99;
		assert!(
			stored
				.save(&acting(&db, grace))
				.await
				.unwrap_err()
				.is_forbidden()
		);
		assert!(
			wallet(ada, "fake")
				.insert(&acting(&db, grace))
				.await
				.unwrap_err()
				.is_forbidden()
		);
		assert_eq!(
			Wallet::update()
				.set(|w| w.balance, 5)
				.run(&acting(&db, grace))
				.await
				.unwrap(),
			0
		);
		// A unique conflict fails the insert, and its entry with it.
		assert!(
			wallet(ada, "ada")
				.insert(&acting(&db, ada))
				.await
				.unwrap_err()
				.is_conflict()
		);

		let tx = acting(&db, ada).begin().await.unwrap();
		wallet(ada, "rolled-back").insert(&tx).await.unwrap();
		tx.rollback().await.unwrap();

		assert_eq!(log(&db).await.len(), 1);
	})
	.await;
}

#[tokio::test]
async fn bulk_writes_and_cascades_record_every_row() {
	on_both(async |db| {
		let ada = Uuid::now_v7();
		let as_ada = acting(&db, ada);
		let mut wallets = Vec::new();

		for name in ["one", "two", "three"] {
			let stored = wallet(ada, name).insert(&as_ada).await.unwrap();
			for amount in [1, 2] {
				NewPayment {
					wallet_id: stored.id,
					amount,
				}
				.insert(&as_ada)
				.await
				.unwrap();
			}
			wallets.push(stored);
		}

		let before = log(&db).await.len();
		assert_eq!(before, 9);

		let changed = Wallet::update()
			.filter(|w| w.name.ne("three"))
			.set(|w| w.balance, 7)
			.run(&as_ada)
			.await
			.unwrap();
		assert_eq!(changed, 2);
		let entries = log(&db).await;
		let updates: Vec<&AuditEntry> = entries[before..].iter().collect();
		assert_eq!(updates.len(), 2);
		assert!(updates.iter().all(|entry| {
			entry.op == AuditOp::Update
				&& field(entry.before.as_ref(), "balance") == Some(&serde_json::json!(0))
				&& field(entry.after.as_ref(), "balance") == Some(&serde_json::json!(7))
		}));

		// Soft-deleting a wallet soft-deletes its payments, and records each.
		let deleted = Wallet::query()
			.filter(|w| w.name.eq("one"))
			.delete(&as_ada)
			.await
			.unwrap();
		assert_eq!(deleted, 1);
		let entries = log(&db).await;
		let recorded = &entries[before + 2..];
		assert_eq!(recorded.len(), 3);
		assert!(recorded.iter().all(|entry| {
			entry.op == AuditOp::Delete && entry.after.is_some() && entry.before.is_some()
		}));
		assert_eq!(
			recorded
				.iter()
				.filter(|entry| entry.table_name == "billing.payments")
				.count(),
			2
		);
		assert!(
			recorded
				.iter()
				.all(|entry| entry.actor() == Some(Actor::from_uuid(ada)))
		);

		// A single soft delete cascades and records too.
		let mut two = Wallet::get(&db, wallets[1].id).await.unwrap();
		two.delete(&as_ada).await.unwrap();
		assert_eq!(log(&db).await.len(), before + 2 + 3 + 3);

		// Removing for good records each row as it was.
		let destroyed = Payment::query()
			.with_deleted()
			.destroy(&as_ada)
			.await
			.unwrap();
		assert_eq!(destroyed, 6);
		let entries = log(&db).await;
		let removed = &entries[before + 8..];
		assert_eq!(removed.len(), 6);
		assert!(removed.iter().all(|entry| {
			entry.op == AuditOp::Delete && entry.after.is_none() && entry.before.is_some()
		}));
	})
	.await;
}

#[tokio::test]
async fn reading_the_log_fails_closed() {
	on_both(async |db| {
		let ada = Uuid::now_v7();
		let grace = Uuid::now_v7();
		wallet(ada, "ada").insert(&acting(&db, ada)).await.unwrap();
		wallet(grace, "grace")
			.insert(&acting(&db, grace))
			.await
			.unwrap();

		let reading = |cx: Context| db.with_context(cx);
		assert_eq!(AuditEntry::query().count(&db).await.unwrap(), 0);
		assert_eq!(
			AuditEntry::query().count(&acting(&db, ada)).await.unwrap(),
			0
		);
		assert_eq!(
			AuditEntry::query()
				.count(&reading(Context::new().with(AuditReader::everything())))
				.await
				.unwrap(),
			2
		);
		let own = AuditReader::matching(|entry, cx| {
			match cx.actor() {
				Some(actor) => entry.actor.eq(actor.uuid()),
				None => Expr::never(),
			}
		});
		let as_ada_reader = reading(Context::new().with(Actor::from_uuid(ada)).with(own));
		let mine = AuditEntry::query().all(&as_ada_reader).await.unwrap();
		assert_eq!(mine.len(), 1);
		assert_eq!(mine[0].actor, Some(ada));
		assert_eq!(
			AuditEntry::query()
				.count(&reading(Context::new().with(own)))
				.await
				.unwrap(),
			0
		);

		// No request may change or remove entries, even one that reads them.
		let reader = reading(Context::new().with(AuditReader::everything()));
		assert_eq!(AuditEntry::query().destroy(&reader).await.unwrap(), 0);
		assert_eq!(AuditEntry::query().delete(&reader).await.unwrap(), 0);
		assert_eq!(log(&db).await.len(), 2);

		// Purging old entries is a deliberate bypass.
		assert_eq!(
			AuditEntry::query()
				.destroy(&db.system("retention"))
				.await
				.unwrap(),
			2
		);
		assert!(log(&db).await.is_empty());
		assert!(matches!(
			AuditEntry::for_table::<Wallet>().count(&db).await,
			Ok(0)
		));
	})
	.await;
}

/// One write in a random history.
#[derive(Clone, Debug)]
enum Op {
	Insert { actor: usize },
	Rename { actor: usize, wallet: usize },
	Delete { actor: usize, wallet: usize },
	Bulk { actor: usize, balance: i64 },
	Purge { actor: usize },
	System { wallet: usize },
}

const ACTORS: usize = 3;

fn op() -> impl Strategy<Value = Op> {
	let actor = 0..ACTORS;
	prop_oneof![
		3 => actor.clone().prop_map(|actor| Op::Insert { actor }),
		3 => (actor.clone(), 0..6_usize).prop_map(|(actor, wallet)| Op::Rename { actor, wallet }),
		2 => (actor.clone(), 0..6_usize).prop_map(|(actor, wallet)| Op::Delete { actor, wallet }),
		2 => (actor.clone(), 0..100_i64).prop_map(|(actor, balance)| Op::Bulk { actor, balance }),
		1 => actor.prop_map(|actor| Op::Purge { actor }),
		1 => (0..6_usize).prop_map(|wallet| Op::System { wallet }),
	]
}

/// A wallet as the model sees it.
#[derive(Clone, Debug)]
struct Model {
	id: Id<Wallet>,
	owner: usize,
	name: String,
	balance: i64,
	deleted: bool,
}

/// What the model expects the log to say about a row, in order.
#[derive(Clone, Debug, PartialEq)]
struct Expected {
	op: AuditOp,
	actor: Option<Uuid>,
	reason: Option<String>,
	before: Option<(String, i64, bool)>,
	after: Option<(String, i64, bool)>,
}

fn snapshot(row: Option<&leptoskit_db::Json<serde_json::Value>>) -> Option<(String, i64, bool)> {
	let row = &row?.0;
	Some((
		row["name"].as_str()?.to_string(),
		row["balance"].as_i64()?,
		!row["deleted_at"].is_null(),
	))
}

/// The wallets and the log, as the model expects them.
struct History<'a> {
	db: &'a Db,
	system: Db,
	actors: Vec<Uuid>,
	rows: Vec<Model>,
	expected: HashMap<Id<Wallet>, Vec<Expected>>,
}

fn state(row: &Model) -> (String, i64, bool) {
	(row.name.clone(), row.balance, row.deleted)
}

impl History<'_> {
	fn as_actor(&self, actor: usize) -> Db {
		acting(self.db, self.actors[actor])
	}

	/// Expects an entry about `row` by `actor`, from `before` to `row` as
	/// it is now, or to nothing.
	fn expect(
		&mut self,
		row: &Model,
		op: AuditOp,
		actor: Option<usize>,
		before: Option<(String, i64, bool)>,
		gone: bool,
	) {
		let reason = actor
			.is_none()
			.then(|| "support adjusted a balance".to_string());
		let entry = Expected {
			op,
			actor: actor.map(|actor| self.actors[actor]),
			reason,
			before,
			after: (!gone).then(|| state(row)),
		};
		self.expected.entry(row.id).or_default().push(entry);
	}

	async fn stored(&self, index: usize) -> Option<Wallet> {
		let id = self.rows.get(index)?.id;
		Wallet::query()
			.with_deleted()
			.filter(|w| w.id.eq(id))
			.first(&self.system)
			.await
			.unwrap()
	}

	async fn insert(&mut self, step: usize, actor: usize) {
		let name = format!("w{step}");
		let stored = wallet(self.actors[actor], &name)
			.insert(&self.as_actor(actor))
			.await
			.unwrap();
		let row = Model {
			id: stored.id,
			owner: actor,
			name,
			balance: 0,
			deleted: false,
		};
		self.expect(&row, AuditOp::Insert, Some(actor), None, false);
		self.rows.push(row);
	}

	async fn rename(
		&mut self,
		step: usize,
		actor: usize,
		index: usize,
	) -> Result<(), TestCaseError> {
		let Some(mut stored) = self.stored(index).await else {
			return Ok(());
		};
		stored.name = format!("r{step}");
		let result = stored.save(&self.as_actor(actor)).await;

		if self.rows[index].owner != actor {
			prop_assert!(
				matches!(result, Err(DbError::Forbidden { .. })),
				"{result:?}"
			);
			return Ok(());
		}

		prop_assert!(result.is_ok(), "{result:?}");
		let before = state(&self.rows[index]);
		self.rows[index].name = format!("r{step}");
		let row = self.rows[index].clone();
		self.expect(&row, AuditOp::Update, Some(actor), Some(before), false);
		Ok(())
	}

	async fn delete(&mut self, actor: usize, index: usize) -> Result<(), TestCaseError> {
		let Some(mut stored) = self.stored(index).await else {
			return Ok(());
		};
		let result = stored.delete(&self.as_actor(actor)).await;

		if self.rows[index].owner != actor {
			prop_assert!(result.is_err());
			return Ok(());
		}

		prop_assert!(result.is_ok(), "{result:?}");
		let before = state(&self.rows[index]);
		self.rows[index].deleted = true;
		let row = self.rows[index].clone();
		self.expect(&row, AuditOp::Delete, Some(actor), Some(before), false);
		Ok(())
	}

	async fn bulk(&mut self, actor: usize, balance: i64) -> Result<(), TestCaseError> {
		let changed = Wallet::update()
			.set(|w| w.balance, balance)
			.run(&self.as_actor(actor))
			.await
			.unwrap();
		let owned: Vec<usize> = (0..self.rows.len())
			.filter(|index| self.rows[*index].owner == actor && !self.rows[*index].deleted)
			.collect();
		prop_assert_eq!(changed, owned.len() as u64);

		for index in owned {
			let before = state(&self.rows[index]);
			self.rows[index].balance = balance;
			let row = self.rows[index].clone();
			self.expect(&row, AuditOp::Update, Some(actor), Some(before), false);
		}

		Ok(())
	}

	async fn purge(&mut self, actor: usize) -> Result<(), TestCaseError> {
		let removed = Wallet::query()
			.with_deleted()
			.destroy(&self.as_actor(actor))
			.await
			.unwrap();
		let (gone, kept): (Vec<Model>, Vec<Model>) = std::mem::take(&mut self.rows)
			.into_iter()
			.partition(|row| row.owner == actor);
		prop_assert_eq!(removed, gone.len() as u64);

		for row in &gone {
			self.expect(row, AuditOp::Delete, Some(actor), Some(state(row)), true);
		}

		self.rows = kept;
		Ok(())
	}

	async fn system_save(&mut self, index: usize) {
		let Some(mut stored) = self.stored(index).await else {
			return;
		};
		stored.balance += 1;
		stored
			.save(&self.db.system("support adjusted a balance"))
			.await
			.unwrap();
		let before = state(&self.rows[index]);
		self.rows[index].balance += 1;
		let row = self.rows[index].clone();
		self.expect(&row, AuditOp::Update, None, Some(before), false);
	}

	/// The log says exactly what the model expects, row by row, in order,
	/// and each actor reading their own entries sees exactly theirs.
	async fn check(self) -> Result<(), TestCaseError> {
		let entries = log(self.db).await;
		let mut found: HashMap<String, Vec<Expected>> = HashMap::new();

		for entry in &entries {
			prop_assert_eq!(&entry.table_name, "wallets");

			for row in [&entry.before, &entry.after].into_iter().flatten() {
				prop_assert!(row.0.get("pin").is_none());
			}

			found
				.entry(entry.row_key.clone())
				.or_default()
				.push(Expected {
					op: entry.op,
					actor: entry.actor,
					reason: entry.reason.clone(),
					before: snapshot(entry.before.as_ref()),
					after: snapshot(entry.after.as_ref()),
				});
		}

		let expected: HashMap<String, Vec<Expected>> = self
			.expected
			.into_iter()
			.map(|(id, history)| (format!("[\"{id}\"]"), history))
			.collect();
		prop_assert_eq!(&found, &expected);

		let own = AuditReader::matching(|entry, cx| {
			match cx.actor() {
				Some(actor) => entry.actor.eq(actor.uuid()),
				None => Expr::never(),
			}
		});

		for actor in &self.actors {
			let reader = self
				.db
				.with_context(Context::new().with(Actor::from_uuid(*actor)).with(own));
			let seen = AuditEntry::query().count(&reader).await.unwrap();
			let theirs = entries
				.iter()
				.filter(|entry| entry.actor == Some(*actor))
				.count() as u64;
			prop_assert_eq!(seen, theirs);
			prop_assert_eq!(
				AuditEntry::query()
					.count(&acting(self.db, *actor))
					.await
					.unwrap(),
				0
			);
		}

		Ok(())
	}
}

async fn agrees_with_the_model(db: &Db, ops: Vec<Op>) -> Result<(), TestCaseError> {
	let system = db.system("resetting the property");
	Payment::query().destroy(&system).await.unwrap();
	Wallet::query().destroy(&system).await.unwrap();
	AuditEntry::query().destroy(&system).await.unwrap();

	let mut history = History {
		db,
		system,
		actors: (0..ACTORS).map(|_| Uuid::now_v7()).collect(),
		rows: Vec::new(),
		expected: HashMap::new(),
	};

	for (step, op) in ops.into_iter().enumerate() {
		match op {
			Op::Insert { actor } => history.insert(step, actor).await,
			Op::Rename { actor, wallet } => history.rename(step, actor, wallet).await?,
			Op::Delete { actor, wallet } => history.delete(actor, wallet).await?,
			Op::Bulk { actor, balance } => history.bulk(actor, balance).await?,
			Op::Purge { actor } => history.purge(actor).await?,
			Op::System { wallet } => history.system_save(wallet).await,
		}
	}

	history.check().await
}

fn run_property(db: &Db, runtime: &tokio::runtime::Runtime, cases: u32) {
	let mut runner = TestRunner::new(Config {
		cases,
		failure_persistence: None,
		..Config::default()
	});
	let outcome = runner.run(&proptest::collection::vec(op(), 1..14), |ops| {
		runtime.block_on(agrees_with_the_model(db, ops))
	});

	if let Err(error) = outcome {
		panic!("{error}");
	}
}

#[test]
fn the_log_agrees_with_the_model_on_sqlite() {
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let db = runtime.block_on(async {
		let db = Db::connect("sqlite::memory:").await.unwrap();
		create_schema(&db).await;
		db
	});
	run_property(&db, &runtime, 96);
}

#[test]
fn the_log_agrees_with_the_model_on_postgres() {
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
