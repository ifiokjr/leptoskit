//! Polymorphic tables against both databases: variants written with their
//! supertype rows, read through the enum one table at a time, the
//! supertype's policies inherited by every variant, soft deletes and
//! removals that follow the supertype, live changes, migrations, and
//! row-level security on Postgres.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use std::time::Duration;

use leptoskit_db::Connection;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Dialect;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Polymorphic;
use leptoskit_db::Read;
use leptoskit_db::Setting;
use leptoskit_db::Table;
use leptoskit_db::Timestamp;
use leptoskit_db::Value;
use leptoskit_db::Variant;
use leptoskit_db::live::ChangedRows;
use leptoskit_db::live::Feed;
use leptoskit_db::live::Invalidation;
use leptoskit_db::policy;
use leptoskit_db::polymorphic;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest::test_runner::TestCaseError;
use proptest::test_runner::TestRunner;
use support::postgres::PostgresCluster;

/// The actor a request acts as.
pub struct Viewer(pub Id<Actor>);

impl Setting for Viewer {
	type Value = Id<Actor>;

	const NAME: &'static str = "viewer";

	fn value(&self) -> Id<Actor> {
		self.0
	}
}

leptoskit_db::register_setting!(Viewer);

/// Anything that can own a budget. Bots are hidden from everyone but
/// themselves, and only an actor may remove itself.
#[polymorphic(name = "actors", public, live)]
#[policy(select, |actor, cx| !actor.kind.eq(ActorKind::Bot) | actor.id.eq(cx.setting::<Viewer>()))]
#[policy(insert, update, |_, _| Expr::always())]
#[policy(delete, |actor, cx| actor.id.eq(cx.setting::<Viewer>()),
	message = "Only an actor can remove itself")]
pub enum Actor {
	/// Someone.
	Person(Person),
	Team(Team),
	#[db(rename = "robot")]
	Bot(Bot),
}

/// A person: anyone sees a listed one.
#[table(name = "people", public, extends = Actor)]
#[policy(select, |person, _| person.listed.eq(true))]
#[policy(insert, update, delete, |_, _| Expr::always())]
pub struct Person {
	pub id: Id<Self>,
	pub name: String,
	pub listed: bool,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// A team: anyone sees one with members.
#[table(name = "teams", public, extends = Actor)]
#[policy(select, |team, _| team.size.gt(0))]
#[policy(insert, update, delete, |_, _| Expr::always())]
pub struct Team {
	pub id: Id<Self>,
	pub title: String,
	pub size: i32,
}

/// A bot: no rules of its own, only the supertype's.
#[table(name = "bots", public, extends = Actor, audit)]
pub struct Bot {
	pub id: Id<Self>,
	pub model: String,
}

/// A budget, owned by an actor of any kind.
#[table(name = "budgets", public)]
pub struct Budget {
	pub id: Id<Self>,
	#[db(backlink = budgets, on_soft_delete = cascade)]
	pub owner_id: Id<Actor>,
	pub reviewer_id: Option<Id<Actor>>,
	pub name: String,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
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

	if let Some(cluster) = PostgresCluster::start() {
		let db = Db::connect(&cluster.database()).await.unwrap();
		create_schema(&db).await;
		check(db).await;
	}
}

fn as_actor(db: &Db, actor: Id<Actor>) -> Db {
	db.with_context(Context::new().with(Viewer(actor)))
}

async fn person(db: &impl Connection, name: &str, listed: bool) -> Person {
	NewPerson {
		name: name.into(),
		listed,
	}
	.insert(db)
	.await
	.unwrap()
}

async fn team(db: &impl Connection, title: &str, size: i32) -> Team {
	NewTeam {
		title: title.into(),
		size,
	}
	.insert(db)
	.await
	.unwrap()
}

async fn bot(db: &impl Connection, model: &str) -> Bot {
	NewBot {
		model: model.into(),
	}
	.insert(db)
	.await
	.unwrap()
}

async fn budget(db: &impl Connection, owner: Id<Actor>, name: &str) -> Budget {
	NewBudget {
		owner_id: owner,
		reviewer_id: None,
		name: name.into(),
	}
	.insert(db)
	.await
	.unwrap()
}

/// What each row is, by the enum's own match: adding a variant breaks it.
fn describe(actor: &Actor) -> String {
	match actor {
		Actor::Person(person) => format!("person {}", person.name),
		Actor::Team(team) => format!("team {}", team.title),
		Actor::Bot(bot) => format!("bot {}", bot.model),
	}
}

fn described(actors: &[Actor]) -> Vec<String> {
	actors.iter().map(describe).collect()
}

/// The supertype rows, as stored.
async fn supertype_rows(db: &Db) -> Vec<(Id<Actor>, ActorKind)> {
	let mut rows: Vec<(Id<Actor>, ActorKind)> = ActorRow::query()
		.all(&db.system("reading the supertype"))
		.await
		.unwrap()
		.into_iter()
		.map(|row| (row.id, row.kind))
		.collect();
	rows.sort();
	rows
}

fn message(error: &DbError) -> String {
	match error {
		DbError::Forbidden { message, .. } => message.clone(),
		other => panic!("expected a refusal, got {other:?}"),
	}
}

#[tokio::test]
async fn variants_write_their_supertype_rows_and_read_back_as_the_enum() {
	on_both(async |db| {
		let system = db.system("a polymorphic test");
		let ada = person(&system, "ada", true).await;
		let core = team(&system, "core", 3).await;
		let helper = bot(&system, "helper").await;

		// One supertype row per variant row, with its kind.
		let mut expected = vec![
			(ada.id.upcast(), ActorKind::Person),
			(core.id.upcast(), ActorKind::Team),
			(helper.id.upcast(), ActorKind::Bot),
		];
		expected.sort();
		assert_eq!(supertype_rows(&db).await, expected);
		assert_eq!(ActorKind::Bot.as_str(), "robot");
		assert_eq!(<Bot as Variant>::KIND, ActorKind::Bot);
		assert_eq!(Actor::KINDS.len(), 3);
		assert_eq!(Actor::table_of(ActorKind::Team).to_string(), "teams");

		// Read through the enum: every kind, in creation order.
		let everyone = Actor::query().all(&system).await.unwrap();
		assert_eq!(
			described(&everyone),
			["person ada", "team core", "bot helper"]
		);
		assert_eq!(
			everyone.iter().map(Actor::kind).collect::<Vec<_>>(),
			[ActorKind::Person, ActorKind::Team, ActorKind::Bot]
		);
		assert_eq!(Actor::query().count(&system).await.unwrap(), 3);
		assert_eq!(
			described(
				&Actor::query()
					.only([ActorKind::Bot, ActorKind::Person, ActorKind::Bot])
					.all(&system)
					.await
					.unwrap()
			),
			["person ada", "bot helper"]
		);
		assert_eq!(
			described(&Actor::query().descending().all(&system).await.unwrap()),
			["bot helper", "team core", "person ada"]
		);

		// Pages over the merged rows.
		let page = |query: leptoskit_db::Variants<Actor>| {
			let system = system.clone();
			async move { described(&query.all(&system).await.unwrap()) }
		};
		assert_eq!(
			page(Actor::query().limit(2)).await,
			["person ada", "team core"]
		);
		assert_eq!(page(Actor::query().offset(1).limit(1)).await, ["team core"]);
		assert_eq!(
			page(Actor::query().after(ada.id.upcast())).await,
			["team core", "bot helper"]
		);
		assert_eq!(
			page(
				Actor::query()
					.descending()
					.after(helper.id.upcast())
					.limit(1)
			)
			.await,
			["team core"]
		);
		assert_eq!(
			page(Actor::query().ids([helper.id.upcast(), ada.id.upcast()])).await,
			["person ada", "bot helper"]
		);
		assert_eq!(
			Actor::query()
				.ids([core.id.upcast()])
				.count(&system)
				.await
				.unwrap(),
			1
		);
		assert_eq!(
			describe(&Actor::query().first(&system).await.unwrap().unwrap()),
			"person ada"
		);

		// One key, of whichever kind.
		let found = Actor::get(&system, core.id.upcast()).await.unwrap();
		assert_eq!(found.id(), core.id.upcast());
		assert_eq!(describe(&found), "team core");
		assert!(matches!(
			Actor::get(&system, Id::new()).await,
			Err(DbError::NotFound { table }) if table == "actors"
		));
		assert_eq!(
			core.id
				.upcast()
				.downcast::<Team>(&system)
				.await
				.unwrap()
				.map(|team| team.title),
			Some("core".to_string())
		);
		assert!(
			core.id
				.upcast()
				.downcast::<Person>(&system)
				.await
				.unwrap()
				.is_none()
		);
	})
	.await;
}

#[tokio::test]
async fn links_to_the_enum_load_whichever_kind_they_point_at() {
	on_both(async |db| {
		let system = db.system("a polymorphic test");
		let ada = person(&system, "ada", true).await;
		let core = team(&system, "core", 3).await;
		let found = Actor::get(&system, core.id.upcast()).await.unwrap();

		// Links to the enum load whichever kind they point at.
		let mut owned = NewBudget {
			owner_id: core.id.upcast(),
			reviewer_id: Some(ada.id.upcast()),
			name: "q3".into(),
		}
		.insert(&system)
		.await
		.unwrap();
		assert_eq!(describe(&owned.owner(&system).await.unwrap()), "team core");
		assert_eq!(
			owned
				.reviewer(&system)
				.await
				.unwrap()
				.map(|reviewer| describe(&reviewer)),
			Some("person ada".to_string())
		);
		owned.reviewer_id = None;
		owned.save(&system).await.unwrap();
		assert!(owned.reviewer(&system).await.unwrap().is_none());
		assert_eq!(
			Budget::by_owner(core.id.upcast())
				.count(&system)
				.await
				.unwrap(),
			1
		);
		assert_eq!(found.budgets().count(&system).await.unwrap(), 1);

		// A budget can't point at a key no actor has.
		assert!(
			NewBudget {
				owner_id: Id::new(),
				reviewer_id: None,
				name: "orphan".into(),
			}
			.insert(&system)
			.await
			.is_err()
		);

		// What a client gets: each variant's public type, tagged by kind.
		let public = PublicActor::from(found);
		assert_eq!(public.kind(), ActorKind::Team);
		let json = serde_json::to_value(&public).unwrap();
		assert_eq!(json["kind"], "team");
		assert_eq!(json["row"]["title"], "core");
		assert_eq!(serde_json::from_value::<PublicActor>(json).unwrap(), public);
		let robot =
			serde_json::to_value(PublicActor::from(Actor::from(bot(&system, "r2").await))).unwrap();
		assert_eq!(robot["kind"], "robot");
		assert!(format!("{:?}", Actor::from(ada)).starts_with("Person(Person {"));
	})
	.await;
}

#[tokio::test]
async fn writes_through_the_enum_reach_the_variant() {
	on_both(async |db| {
		let system = db.system("a polymorphic test");
		let mut actor = Actor::from(team(&system, "core", 3).await);

		if let Actor::Team(team) = &mut actor {
			team.size = 5;
		}

		actor.save(&system).await.unwrap();
		let Actor::Team(stored) = Actor::get(&system, actor.id()).await.unwrap() else {
			panic!("the row is still a team");
		};
		assert_eq!(stored.size, 5);

		// A variant without a soft-delete column is removed, supertype and all.
		let mut gone = Actor::from(bot(&system, "old").await);
		gone.delete(&system).await.unwrap();
		assert!(Actor::find(&system, gone.id()).await.unwrap().is_none());
		assert_eq!(supertype_rows(&db).await.len(), 1);

		actor.destroy(&system).await.unwrap();
		assert_eq!(
			supertype_rows(&db).await,
			Vec::<(Id<Actor>, ActorKind)>::new()
		);
	})
	.await;
}

#[tokio::test]
async fn removing_a_variant_removes_its_supertype_row_or_nothing() {
	on_both(async |db| {
		let system = db.system("a polymorphic test");
		let ada = person(&system, "ada", true).await;
		let grace = person(&system, "grace", true).await;
		let core = team(&system, "core", 3).await;
		team(&system, "ops", 1).await;
		team(&system, "empty", 0).await;
		budget(&system, grace.id.upcast(), "grace's").await;

		ada.destroy(&system).await.unwrap();
		assert_eq!(supertype_rows(&db).await.len(), 4);

		// Bulk removals remove each removed row's supertype row.
		assert_eq!(
			Team::query()
				.filter(|team| team.size.gt(0))
				.destroy(&system)
				.await
				.unwrap(),
			2
		);
		assert_eq!(supertype_rows(&db).await, {
			let mut rows = vec![
				(grace.id.upcast(), ActorKind::Person),
				(
					Team::query().one(&system).await.unwrap().id.upcast(),
					ActorKind::Team,
				),
			];
			rows.sort();
			rows
		});
		assert!(
			Actor::find(&system, core.id.upcast())
				.await
				.unwrap()
				.is_none()
		);

		// A budget still points at grace's supertype row, so removing her
		// fails, and leaves her as she was.
		assert!(grace.clone_for_test().destroy(&system).await.is_err());
		assert!(Person::find(&system, grace.id).await.unwrap().is_some());
		assert_eq!(supertype_rows(&db).await.len(), 2);
		assert!(
			Person::query()
				.with_deleted()
				.destroy(&system)
				.await
				.is_err()
		);
		assert_eq!(Person::query().count(&system).await.unwrap(), 1);

		// Removing the supertype row directly removes the variant row with it.
		Budget::query().destroy(&system).await.unwrap();
		ActorRow::query()
			.filter(|row| row.id.eq(grace.id.upcast()))
			.destroy(&system)
			.await
			.unwrap();
		assert_eq!(
			Person::query().with_deleted().count(&system).await.unwrap(),
			0
		);
	})
	.await;
}

impl Person {
	/// A second handle to the same row, for a removal expected to fail.
	fn clone_for_test(&self) -> Self {
		Self {
			id: self.id,
			name: self.name.clone(),
			listed: self.listed,
			deleted_at: self.deleted_at,
		}
	}
}

#[tokio::test]
async fn soft_deletes_hide_variants_and_cascade_through_the_supertype() {
	on_both(async |db| {
		let system = db.system("a polymorphic test");
		let mut ada = person(&system, "ada", true).await;
		let core = team(&system, "core", 2).await;
		let hers = budget(&system, ada.id.upcast(), "hers").await;
		let theirs = budget(&system, core.id.upcast(), "theirs").await;

		ada.delete(&system).await.unwrap();
		assert!(ada.deleted_at.is_some());
		// Her supertype row stays, so links to her still hold.
		assert_eq!(supertype_rows(&db).await.len(), 2);
		assert_eq!(
			described(&Actor::query().all(&system).await.unwrap()),
			["team core"]
		);
		assert_eq!(
			described(&Actor::query().with_deleted().all(&system).await.unwrap()),
			["person ada", "team core"]
		);
		assert_eq!(
			Actor::query().with_deleted().count(&system).await.unwrap(),
			2
		);
		assert!(
			Actor::find(&system, ada.id.upcast())
				.await
				.unwrap()
				.is_none()
		);

		// Budgets cascading from her supertype row went with her.
		assert!(Budget::find(&system, hers.id).await.unwrap().is_none());
		assert!(Budget::find(&system, theirs.id).await.unwrap().is_some());

		// Bulk soft deletes cascade too.
		let mut bea = person(&system, "bea", false).await;
		let bulk = budget(&system, bea.id.upcast(), "bulk").await;
		assert_eq!(
			Person::query()
				.filter(|person| person.name.eq("bea"))
				.delete(&system)
				.await
				.unwrap(),
			1
		);
		assert!(Budget::find(&system, bulk.id).await.unwrap().is_none());

		// A soft-deleted owner is not found through the link.
		bea.deleted_at = None;
		let owned = Budget::query()
			.with_deleted()
			.filter(|budget| budget.id.eq(bulk.id))
			.one(&system)
			.await
			.unwrap();
		assert!(matches!(
			owned.owner(&system).await,
			Err(DbError::NotFound { table }) if table == "actors"
		));
	})
	.await;
}

#[tokio::test]
async fn the_supertypes_policies_govern_every_variant() {
	on_both(async |db| {
		let system = db.system("a polymorphic test");
		let ada = person(&system, "ada", true).await;
		let hidden = person(&system, "hidden", false).await;
		let core = team(&system, "core", 3).await;
		let helper = bot(&system, "helper").await;
		let anonymous = db.with_context(Context::new());

		// The bot is hidden by the supertype's rule, through the enum and
		// through its own table, which has no rules of its own.
		assert_eq!(
			described(&Actor::query().all(&anonymous).await.unwrap()),
			["person ada", "team core"]
		);
		assert_eq!(Bot::query().count(&anonymous).await.unwrap(), 0);
		assert!(
			Actor::find(&anonymous, helper.id.upcast())
				.await
				.unwrap()
				.is_none()
		);
		assert!(
			helper
				.id
				.upcast()
				.downcast::<Bot>(&anonymous)
				.await
				.unwrap()
				.is_none()
		);
		assert!(
			Actor::find(&anonymous, hidden.id.upcast())
				.await
				.unwrap()
				.is_none()
		);
		assert_eq!(Actor::query().count(&anonymous).await.unwrap(), 2);

		// The bot sees itself.
		let itself = as_actor(&db, helper.id.upcast());
		assert_eq!(
			described(&Actor::query().all(&itself).await.unwrap()),
			["person ada", "team core", "bot helper"]
		);

		// Creating a row the request couldn't then see is refused, and
		// rolled back with its supertype row.
		let refused = NewBot {
			model: "sneaky".into(),
		}
		.insert(&anonymous)
		.await
		.unwrap_err();
		assert!(matches!(refused, DbError::Forbidden { ref table, .. } if table == "bots"));
		assert_eq!(supertype_rows(&db).await.len(), 4);

		// Only an actor may remove itself, with the supertype's message.
		let error = ada.clone_for_test().destroy(&anonymous).await.unwrap_err();
		assert_eq!(message(&error), "Only an actor can remove itself");
		let error = Actor::from(ada.clone_for_test())
			.destroy(&as_actor(&db, core.id.upcast()))
			.await
			.unwrap_err();
		assert_eq!(message(&error), "Only an actor can remove itself");
		assert_eq!(
			Person::query()
				.destroy(&as_actor(&db, core.id.upcast()))
				.await
				.unwrap(),
			0
		);
		assert_eq!(supertype_rows(&db).await.len(), 4);

		Actor::from(ada.clone_for_test())
			.destroy(&as_actor(&db, ada.id.upcast()))
			.await
			.unwrap();
		assert_eq!(supertype_rows(&db).await.len(), 3);

		// Updates need the supertype row to be visible.
		let mut hidden_bot = helper;
		hidden_bot.model = "renamed".into();
		assert!(matches!(
			hidden_bot.save(&anonymous).await,
			Err(DbError::NotFound { .. })
		));
		hidden_bot
			.save(&as_actor(&db, hidden_bot.id.upcast()))
			.await
			.unwrap();
	})
	.await;
}

/// The next change, or a failure after a few seconds.
async fn next(feed: &mut Feed) -> Invalidation {
	tokio::time::timeout(Duration::from_secs(10), feed.next())
		.await
		.expect("a change arrives")
		.unwrap()
}

fn actors_changed(keys: &[Id<Actor>]) -> Invalidation {
	Invalidation::Rows(vec![ChangedRows {
		table: "actors".into(),
		keys: keys.iter().map(|key| vec![key.to_string()]).collect(),
	}])
}

#[tokio::test]
async fn a_live_supertype_publishes_each_variant_row_it_gains_and_loses() {
	on_both(async |db| {
		let system = db.system("a polymorphic test");
		let mut feed = system.changes().await.unwrap();
		let mut anonymous = db.changes().await.unwrap();

		let ada = person(&system, "ada", true).await;
		assert_eq!(next(&mut feed).await, actors_changed(&[ada.id.upcast()]));
		assert_eq!(
			next(&mut anonymous).await,
			actors_changed(&[ada.id.upcast()])
		);

		// The anonymous subscriber can't read the bot's supertype row, so
		// it never hears of it.
		let helper = bot(&system, "helper").await;
		let helper_id = helper.id.upcast();
		assert_eq!(next(&mut feed).await, actors_changed(&[helper_id]));
		helper.destroy(&system).await.unwrap();
		assert_eq!(next(&mut feed).await, actors_changed(&[helper_id]));

		let ada_id = ada.id.upcast();
		ada.destroy(&system).await.unwrap();
		assert_eq!(next(&mut feed).await, actors_changed(&[ada_id]));
		assert_eq!(next(&mut anonymous).await, actors_changed(&[ada_id]));
	})
	.await;
}

#[test]
fn migrations_create_the_supertype_and_its_foreign_keys() {
	for dialect in [Dialect::Postgres, Dialect::Sqlite] {
		let schema = leptoskit_db::schema(dialect).unwrap();
		let actors = schema
			.tables
			.iter()
			.find(|table| table.name.name == "actors")
			.unwrap();
		assert_eq!(
			actors
				.columns
				.iter()
				.map(|column| column.name.as_str())
				.collect::<Vec<_>>(),
			["id", "kind"]
		);
		assert_eq!(
			actors.foreign_keys,
			Vec::<leptoskit_db_schema::ForeignKey>::new()
		);

		let empty = Schema::empty(dialect);
		let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
		let migration = leptoskit_db_schema::render(&plan, &empty, &schema);
		let name = match dialect {
			Dialect::Postgres => "migration_postgres",
			Dialect::Sqlite => "migration_sqlite",
		};
		insta::assert_snapshot!(name, migration.sql());
	}

	let people = Person::definition();
	let key = people
		.foreign_keys
		.iter()
		.find(|key| key.columns == ["id"])
		.unwrap();
	assert_eq!(key.references.name, "actors");
	assert_eq!(
		key.on_delete,
		leptoskit_db_schema::ReferentialAction::Cascade
	);
	assert!(<Person as Table>::SUPERTYPE.is_some());
	assert!(<Budget as Table>::SUPERTYPE.is_none());
}

/// A role that owns the database but isn't a superuser: Postgres exempts
/// superusers from row-level security.
const ROLE: &str = "lk_polymorphic";

async fn owned_database(cluster: &PostgresCluster) -> String {
	let url = cluster.database();
	let name = url
		.rsplit('/')
		.next()
		.and_then(|tail| tail.split('?').next())
		.unwrap()
		.to_string();
	let admin = Db::connect(&url).await.unwrap();
	admin
		.run_script(&format!(
			"DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = '{ROLE}') THEN \
			 CREATE ROLE {ROLE} LOGIN; END IF; END $$; ALTER DATABASE {name} OWNER TO {ROLE}; \
			 ALTER SCHEMA public OWNER TO {ROLE}"
		))
		.await
		.unwrap();
	url.replacen("postgres@", &format!("{ROLE}@"), 1)
}

async fn create(db: &Db, schema: &Schema) {
	let empty = Schema::empty(schema.dialect);
	let plan = leptoskit_db_schema::diff(&empty, schema, &mut RequireHints).unwrap();
	let migration = leptoskit_db_schema::render(&plan, &empty, schema);
	db.run_script(&migration.sql()).await.unwrap();
}

#[test]
fn variants_inherit_the_supertype_as_row_level_security() {
	let schema = leptoskit_db::schema_with_row_security(Dialect::Postgres).unwrap();
	let security = |name: &str| {
		schema
			.tables
			.iter()
			.find(|table| table.name.name == name)
			.unwrap()
			.row_security
			.clone()
			.unwrap()
	};
	// Bots have no rules of their own, but inherit the supertype's.
	let bots = security("bots");
	let select = bots
		.policies
		.iter()
		.find(|policy| policy.command == leptoskit_db_schema::PolicyCommand::Select)
		.unwrap();
	assert_eq!(
		select.using.as_deref(),
		Some(
			"\"id\" IN (SELECT \"id\" FROM \"actors\" AS \"lk_in_1\" WHERE ((NOT (\"kind\" = \
			 E'robot') OR \"id\" = NULLIF(current_setting('leptoskit.viewer', true), '')::uuid)))"
		)
	);
	assert!(
		security("people").policies[0]
			.using
			.as_deref()
			.unwrap()
			.starts_with("(\"listed\" = TRUE AND \"id\" IN (SELECT")
	);
	assert!(
		schema
			.tables
			.iter()
			.find(|table| table.name.name == "budgets")
			.unwrap()
			.row_security
			.is_none()
	);

	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let runtime = tokio::runtime::Runtime::new().unwrap();
	runtime.block_on(async {
		let db = Db::connect(&owned_database(&cluster).await).await.unwrap();
		create(&db, &schema).await;
		let db = db.with_row_security().unwrap();
		let system = db.system("a polymorphic test");
		let ada = person(&system, "ada", true).await;
		let helper = bot(&system, "helper").await;
		let anonymous = db.with_context(Context::new());

		// Raw SQL obeys the inherited rule too.
		let raw = async |connection: &Db, sql: &str| -> Vec<Vec<Value>> {
			connection
				.fetch(leptoskit_db::Statement {
					sql: sql.into(),
					args: Vec::new(),
				})
				.await
				.unwrap()
		};
		assert_eq!(
			raw(&anonymous, "SELECT id FROM bots").await,
			Vec::<Vec<Value>>::new()
		);
		assert_eq!(
			raw(&as_actor(&db, helper.id.upcast()), "SELECT id FROM bots")
				.await
				.len(),
			1
		);
		assert_eq!(
			described(&Actor::query().all(&anonymous).await.unwrap()),
			["person ada"]
		);

		// The database layer still refuses first, with the rule's message.
		let refused = NewBot {
			model: "sneaky".into(),
		}
		.insert(&anonymous)
		.await
		.unwrap_err();
		assert!(matches!(refused, DbError::Forbidden { ref table, .. } if table == "bots"));
		let error = ada.clone_for_test().destroy(&anonymous).await.unwrap_err();
		assert_eq!(message(&error), "Only an actor can remove itself");
		Actor::from(ada.clone_for_test())
			.destroy(&as_actor(&db, ada.id.upcast()))
			.await
			.unwrap();
		assert_eq!(supertype_rows(&db).await.len(), 1);
	});
}

/// How many rows of each kind a property makes.
const ROWS: usize = 6;

/// One row: its kind (0 person, 1 team, 2 bot), whether a person is
/// listed, a team's size, and whether a person is soft-deleted.
type RowSpec = (u8, bool, i32, bool);

#[derive(Clone, Debug)]
struct Spec {
	rows: Vec<RowSpec>,
	/// Which row the request acts as, if any.
	viewer: Option<usize>,
	only: Vec<u8>,
	descending: bool,
	limit: Option<u64>,
	offset: u64,
}

fn spec() -> impl Strategy<Value = Spec> {
	(
		proptest::collection::vec((0..3u8, any::<bool>(), -1..3i32, any::<bool>()), 0..ROWS),
		proptest::option::of(0..ROWS),
		proptest::collection::vec(0..3u8, 0..3),
		any::<bool>(),
		proptest::option::of(0..4u64),
		0..3u64,
	)
		.prop_map(|(rows, viewer, only, descending, limit, offset)| {
			Spec {
				rows,
				viewer,
				only,
				descending,
				limit,
				offset,
			}
		})
}

const KINDS: [ActorKind; 3] = [ActorKind::Person, ActorKind::Team, ActorKind::Bot];

/// Who may see a row, worked out without the database: the supertype's
/// rule, then the variant's, then soft deletes.
fn visible(row: RowSpec, id: Id<Actor>, viewer: Option<Id<Actor>>) -> bool {
	let (kind, listed, size, deleted) = row;
	let supertype = kind != 2 || viewer == Some(id);
	let variant = match kind {
		0 => listed && !deleted,
		1 => size > 0,
		_ => true,
	};
	supertype && variant
}

/// The rows a request reads through the enum, and how many it counts, as
/// the database says and as [`visible`] says.
async fn compare(
	db: &Db,
	spec: &Spec,
	rule: fn(RowSpec, Id<Actor>, Option<Id<Actor>>) -> bool,
) -> Result<(), TestCaseError> {
	let system = db.system("resetting the property");
	Budget::query().destroy(&system).await.unwrap();
	Person::query().destroy(&system).await.unwrap();
	Team::query().destroy(&system).await.unwrap();
	Bot::query().destroy(&system).await.unwrap();

	let mut ids = Vec::new();

	for (index, row) in spec.rows.iter().enumerate() {
		let name = format!("r{index}");
		let id = match row.0 {
			0 => {
				let mut person = person(&system, &name, row.1).await;

				if row.3 {
					person.delete(&system).await.unwrap();
				}

				person.id.upcast()
			}
			1 => team(&system, &name, row.2).await.id.upcast(),
			_ => bot(&system, &name).await.id.upcast(),
		};
		ids.push(id);
	}

	let viewer = spec.viewer.and_then(|index| ids.get(index).copied());
	let request = match viewer {
		Some(viewer) => as_actor(db, viewer),
		None => db.with_context(Context::new()),
	};
	let only: Vec<ActorKind> = spec
		.only
		.iter()
		.map(|kind| KINDS[usize::from(*kind)])
		.collect();
	let mut query = Actor::query();

	if !only.is_empty() {
		query = query.only(only.iter().copied());
	}

	if spec.descending {
		query = query.descending();
	}

	if let Some(limit) = spec.limit {
		query = query.limit(limit);
	}

	let read: Vec<Id<Actor>> = query
		.offset(spec.offset)
		.all(&request)
		.await
		.unwrap()
		.iter()
		.map(Actor::id)
		.collect();

	let mut expected: Vec<Id<Actor>> = spec
		.rows
		.iter()
		.zip(&ids)
		.filter(|(row, id)| {
			(only.is_empty() || only.contains(&KINDS[usize::from(row.0)]))
				&& rule(**row, **id, viewer)
		})
		.map(|(_, id)| *id)
		.collect();
	expected.sort();

	if spec.descending {
		expected.reverse();
	}

	let counted = expected.len() as u64;
	let expected: Vec<Id<Actor>> = expected
		.into_iter()
		.skip(usize::try_from(spec.offset).unwrap())
		.take(
			spec.limit
				.map_or(usize::MAX, |limit| usize::try_from(limit).unwrap()),
		)
		.collect();
	prop_assert_eq!(read, expected);

	let mut count = Actor::query();

	if !only.is_empty() {
		count = count.only(only.iter().copied());
	}

	prop_assert_eq!(count.count(&request).await.unwrap(), counted);

	for (row, id) in spec.rows.iter().zip(&ids) {
		prop_assert_eq!(
			Actor::find(&request, *id).await.unwrap().is_some(),
			rule(*row, *id, viewer),
			"finding {:?}",
			row
		);
	}

	Ok(())
}

fn runner(cases: u32) -> TestRunner {
	TestRunner::new(Config {
		cases,
		failure_persistence: None,
		..Config::default()
	})
}

fn holds(db: &Db, cases: u32, rule: fn(RowSpec, Id<Actor>, Option<Id<Actor>>) -> bool) -> bool {
	let runtime = tokio::runtime::Handle::current();
	tokio::task::block_in_place(|| {
		runner(cases)
			.run(&spec(), |spec| runtime.block_on(compare(db, &spec, rule)))
			.is_ok()
	})
}

/// [`visible`], forgetting the supertype's rule: a model the property must
/// tell apart from the database.
fn ignoring_the_supertype(row: RowSpec, _: Id<Actor>, _: Option<Id<Actor>>) -> bool {
	let (kind, listed, size, deleted) = row;
	match kind {
		0 => listed && !deleted,
		1 => size > 0,
		_ => true,
	}
}

#[tokio::test(flavor = "multi_thread")]
async fn reading_through_the_enum_shows_exactly_what_the_model_allows() {
	let sqlite = Db::connect("sqlite::memory:").await.unwrap();
	create_schema(&sqlite).await;
	assert!(holds(&sqlite, 64, visible));
	// The property can fail: a model without the supertype's rule is
	// caught.
	assert!(!holds(&sqlite, 64, ignoring_the_supertype));

	if let Some(cluster) = PostgresCluster::start() {
		let postgres = Db::connect(&cluster.database()).await.unwrap();
		create_schema(&postgres).await;
		assert!(holds(&postgres, 32, visible));
		assert!(!holds(&postgres, 64, ignoring_the_supertype));
	}
}
