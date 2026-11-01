//! Read sets, against both databases: a `Tracked` connection records every
//! read a live query makes, with the conditions that decide which rows it
//! saw, so that `Expr::eval` can tell which writes might change its result.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use std::collections::BTreeMap;

use leptoskit_db as db;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::HasId;
use leptoskit_db::Id;
use leptoskit_db::Read;
use leptoskit_db::ReadEntry;
use leptoskit_db::ReadKind;
use leptoskit_db::Statement;
use leptoskit_db::Table;
use leptoskit_db::Timestamp;
use leptoskit_db::Tracked;
use leptoskit_db::Tri;
use leptoskit_db::Value;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db::view;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;

/// The signed-in person.
struct Viewer(Id<Person>);

#[table(name = "people", public)]
pub struct Person {
	pub id: Id<Self>,
	pub name: String,
}

/// A channel: public ones are for everyone, private ones for members.
#[table(name = "channels", public)]
#[policy(select, |channel, cx| channel.private.eq(false) | cx.global(|viewer: &Viewer| cx.exists(Membership::query()
	.filter(|member| member.channel_id.eq(channel.id) & member.person_id.eq(viewer.0)))))]
pub struct Channel {
	pub id: Id<Self>,
	pub name: String,
	pub private: bool,
}

/// Who is in a channel. People see their own memberships.
#[table(name = "memberships")]
#[policy(select, |member, cx| cx.global(|viewer: &Viewer| member.person_id.eq(viewer.0)))]
pub struct Membership {
	pub id: Id<Self>,
	pub channel_id: Id<Channel>,
	pub person_id: Id<Person>,
}

/// A message, which only the channel's members see.
#[table(name = "messages", public, search(language = "simple"))]
#[policy(select, |message, cx| cx.global(|viewer: &Viewer| cx.exists(Membership::query()
	.filter(|member| member.channel_id.eq(message.channel_id) & member.person_id.eq(viewer.0)))))]
pub struct Message {
	pub id: Id<Self>,
	#[db(backlink = messages)]
	pub channel_id: Id<Channel>,
	pub author_id: Id<Person>,
	#[db(search)]
	pub body: String,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

#[table(name = "labels", public)]
pub struct Label {
	pub id: Id<Self>,
	pub name: String,
}

/// A label on a message.
#[db::link(name = "message_labels", from = Message as labels, to = Label as messages)]
pub struct MessageLabel {}

/// How many messages each channel has, read live from `messages`.
#[view(
	name = "channel_totals",
	reads(Message),
	sql = "SELECT channel_id, count(*) AS messages FROM messages WHERE deleted_at IS NULL GROUP \
	       BY channel_id"
)]
pub struct ChannelTotal {
	pub channel_id: Id<Channel>,
	pub messages: i64,
}

/// The same, without saying what it reads.
#[view(name = "channel_tallies", sql = "SELECT channel_id FROM messages")]
pub struct ChannelTally {
	pub channel_id: Id<Channel>,
}

/// The same, kept until refreshed.
#[view(
	name = "channel_snapshots",
	materialized,
	sql = "SELECT channel_id, count(*) AS messages FROM messages GROUP BY channel_id"
)]
pub struct ChannelSnapshot {
	pub channel_id: Id<Channel>,
	pub messages: i64,
}

#[derive(db::Shape, Debug)]
#[db(from = Channel, public)]
pub struct ChannelCard {
	pub id: Id<Channel>,
	pub name: String,
	#[db(count(messages))]
	pub message_count: i64,
	#[db(via = messages, order_by = |message| message.created_at.desc(), limit = 2)]
	pub latest: Vec<MessageCard>,
}

#[derive(db::Shape, Debug)]
#[db(from = Message, public)]
pub struct MessageCard {
	pub body: String,
	#[db(via = author)]
	pub author: PersonName,
	#[db(via = labels)]
	pub labels: Vec<LabelName>,
}

#[derive(db::Shape, Debug)]
#[db(from = Person, public)]
pub struct PersonName {
	pub name: String,
}

#[derive(db::Shape, Debug)]
#[db(from = Label, public)]
pub struct LabelName {
	pub name: String,
}

async fn create_schema(db: &Db) {
	let dialect = db.dialect();
	let schema = leptoskit_db::schema(dialect).unwrap();
	let empty = Schema::empty(dialect);
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
	let migration = leptoskit_db_schema::render(&plan, &empty, &schema);
	db.run_script(&migration.sql()).await.unwrap();
}

/// A world to read: Ada is in `#general`, Grace in `#secret`, and each
/// channel has messages; one in `#general` is deleted and one is labelled.
struct World {
	db: Db,
	ada: Id<Person>,
	grace: Id<Person>,
	general: Id<Channel>,
	secret: Id<Channel>,
	hello: Id<Message>,
	ada_in_general: Id<Membership>,
	label: Id<Label>,
}

impl World {
	async fn new(db: Db) -> Self {
		create_schema(&db).await;
		let system = db.system("test setup");
		let person = async |name: &str| {
			NewPerson { name: name.into() }
				.insert(&system)
				.await
				.unwrap()
				.id
		};
		let ada = person("Ada").await;
		let grace = person("Grace").await;
		let channel = async |name: &str, private| {
			NewChannel {
				name: name.into(),
				private,
			}
			.insert(&system)
			.await
			.unwrap()
			.id
		};
		let general = channel("general", true).await;
		let secret = channel("secret", true).await;

		let ada_in_general = NewMembership {
			channel_id: general,
			person_id: ada,
		}
		.insert(&system)
		.await
		.unwrap()
		.id;
		NewMembership {
			channel_id: secret,
			person_id: grace,
		}
		.insert(&system)
		.await
		.unwrap();
		let message = async |channel_id, author_id, body: &str| {
			NewMessage {
				channel_id,
				author_id,
				body: body.into(),
			}
			.insert(&system)
			.await
			.unwrap()
		};
		let hello = message(general, ada, "hello").await;
		message(general, ada, "again").await;
		message(general, ada, "gone")
			.await
			.delete(&system)
			.await
			.unwrap();
		message(secret, grace, "psst").await;
		let label = NewLabel {
			name: "important".into(),
		}
		.insert(&system)
		.await
		.unwrap()
		.id;

		hello.labels().add(&system, label, ()).await.unwrap();

		Self {
			db,
			ada,
			grace,
			general,
			secret,
			hello: hello.id,
			ada_in_general,
			label,
		}
	}

	/// A tracked connection acting for `person`.
	fn tracked(&self, person: Id<Person>) -> Tracked<Db> {
		Tracked::new(self.db.with_context(Context::new().with(Viewer(person))))
	}
}

async fn on_both(check: impl AsyncFn(World)) {
	check(World::new(Db::connect("sqlite::memory:").await.unwrap()).await).await;

	if let Some(cluster) = support::postgres::PostgresCluster::start() {
		check(World::new(Db::connect(&cluster.database()).await.unwrap()).await).await;
	}
}

/// A row of `T`, as a change log would hold it.
fn row<T: Table>(row: &T) -> BTreeMap<String, Value> {
	T::COLUMNS
		.iter()
		.map(ToString::to_string)
		.zip(row.to_values().unwrap())
		.collect()
}

/// The entries of `reads` about `T`.
fn about<T: Table>(reads: &[ReadEntry]) -> Vec<&ReadEntry> {
	reads
		.iter()
		.filter(|entry| entry.table == T::table_name())
		.collect()
}

fn keys(ids: &[Id<Message>]) -> Vec<Vec<Value>> {
	ids.iter().map(|id| vec![Value::Uuid(id.uuid())]).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_query_records_its_filter_policy_and_soft_deletes() {
	on_both(async |world: World| {
		let tracked = world.tracked(world.ada);
		let messages = Message::query()
			.filter(|message| message.channel_id.eq(world.general))
			.order_by(|message| message.created_at.asc())
			.limit(10)
			.offset(0)
			.all(&tracked)
			.await
			.unwrap();
		let ids: Vec<Id<Message>> = messages.iter().map(|message| message.id).collect();
		assert_eq!(messages.len(), 2);

		let reads = tracked.into_reads();
		let read = reads[0].clone();
		assert_eq!(read.table, Message::table_name());
		assert_eq!(read.kind, ReadKind::Rows);
		assert_eq!(read.limit, Some(10));
		assert_eq!(read.offset, Some(0));
		assert_eq!(read.order.len(), 1);
		assert_eq!(read.order[0].column(), "created_at");
		assert!(!read.order[0].is_descending());
		assert_eq!(read.keys, Some(keys(&ids)));

		// A message in another channel can't change the result; one in this
		// channel might, depending on memberships; a deleted one can't.
		let mut message = Message::get(&world.db.system("test"), world.hello)
			.await
			.unwrap();
		assert_eq!(read.condition.eval(&row(&message)), Tri::Unknown);
		message.channel_id = world.secret;
		assert_eq!(read.condition.eval(&row(&message)), Tri::False);
		message.channel_id = world.general;
		message.deleted_at = Some(leptoskit_db::now());
		assert_eq!(read.condition.eval(&row(&message)), Tri::False);

		// The membership that lets Ada read is read too, so revoking it is a
		// change to the result; Grace's memberships aren't.
		let memberships = about::<Membership>(&reads);
		assert_eq!(memberships.len(), 1);
		assert_eq!(memberships[0].kind, ReadKind::Count);
		let system = world.db.system("test");
		let mut membership = Membership::get(&system, world.ada_in_general)
			.await
			.unwrap();
		assert!(memberships[0].condition.eval(&row(&membership)).may_match());
		membership.person_id = world.grace;
		assert_eq!(memberships[0].condition.eval(&row(&membership)), Tri::False);
		assert_eq!(reads.len(), 2);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn counts_exists_and_point_reads_say_what_they_read() {
	on_both(async |world: World| {
		let tracked = world.tracked(world.ada);
		let general = Message::query().filter(|message| message.channel_id.eq(world.general));

		assert_eq!(general.count(&tracked).await.unwrap(), 2);
		assert!(
			Message::query()
				.filter(|message| message.id.eq(world.hello))
				.exists(&tracked)
				.await
				.unwrap()
		);
		let hello = Message::get(&tracked, world.hello).await.unwrap();
		assert!(Message::find(&tracked, Id::new()).await.unwrap().is_none());
		let channel = hello.channel(&tracked).await.unwrap();
		assert_eq!(channel.id, world.general);

		let reads = tracked.reads();
		let messages = about::<Message>(&reads);
		let kinds: Vec<ReadKind> = messages.iter().map(|entry| entry.kind).collect();
		assert_eq!(
			kinds,
			[
				ReadKind::Count,
				ReadKind::Exists,
				ReadKind::Point,
				ReadKind::Point
			]
		);
		assert_eq!(messages[0].keys, None);
		assert_eq!(messages[1].keys, Some(keys(&[world.hello])));
		assert_eq!(messages[2].keys, Some(keys(&[world.hello])));
		assert_eq!(messages[2].limit, Some(1));
		assert_eq!(messages[3].keys, Some(Vec::new()));

		let channels = about::<Channel>(&reads);
		assert_eq!(channels.len(), 1);
		assert_eq!(channels[0].kind, ReadKind::Point);
		assert_eq!(
			channels[0].keys,
			Some(vec![vec![Value::Uuid(world.general.uuid())]])
		);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn links_record_the_link_table_and_both_ends() {
	on_both(async |world: World| {
		let tracked = world.tracked(world.ada);
		let hello = Message::get(&world.db.system("test"), world.hello)
			.await
			.unwrap();
		let labels = hello.labels().all(&tracked).await.unwrap();
		assert_eq!(labels.len(), 1);
		assert_eq!(hello.labels().count(&tracked).await.unwrap(), 1);

		let reads = tracked.into_reads();
		let links = about::<MessageLabel>(&reads);
		assert_eq!(links.len(), 2);
		assert_eq!(links[0].kind, ReadKind::Rows);
		assert_eq!(links[1].kind, ReadKind::Count);
		let link = vec![
			Value::Uuid(world.hello.uuid()),
			Value::Uuid(world.label.uuid()),
		];
		assert_eq!(links[0].keys, Some(vec![link]));

		// Whether the message end is visible depends on the message's own
		// policy, which reads memberships.
		let messages = about::<Message>(&reads);
		assert_eq!(messages.len(), 2);
		assert!(messages.iter().all(|entry| entry.kind == ReadKind::Rows));
		assert_eq!(about::<Membership>(&reads).len(), 2);
		let labels = about::<Label>(&reads);
		assert_eq!(labels.len(), 1);
		assert_eq!(
			labels[0].keys,
			Some(vec![vec![Value::Uuid(world.label.uuid())]])
		);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shapes_record_every_level_with_its_own_policy() {
	on_both(async |world: World| {
		let tracked = world.tracked(world.ada);
		let cards: Vec<ChannelCard> = Channel::query()
			.order_by(|channel| channel.name.asc())
			.shape()
			.all(&tracked)
			.await
			.unwrap();
		assert_eq!(cards.len(), 1);
		assert_eq!(cards[0].message_count, 2);
		assert_eq!(cards[0].latest.len(), 2);
		assert_eq!(
			cards[0]
				.latest
				.iter()
				.map(|card| card.labels.len())
				.sum::<usize>(),
			1
		);
		assert_eq!(HasId::<Channel>::id(&cards[0]), world.general);

		let reads = tracked.into_reads();
		let channels = about::<Channel>(&reads);
		assert_eq!(channels.len(), 1);
		assert_eq!(channels[0].keys, None);
		assert_eq!(channels[0].order[0].column(), "name");

		// The count and the latest messages, each joined to its channel: a
		// message anywhere might belong, unless it's deleted.
		let messages = about::<Message>(&reads);
		assert_eq!(
			messages.iter().map(|entry| entry.kind).collect::<Vec<_>>(),
			[ReadKind::Count, ReadKind::Rows]
		);
		assert_eq!(messages[1].limit, Some(2));
		assert!(messages[1].order[0].is_descending());
		let mut gone = Message::get(&world.db.system("test"), world.hello)
			.await
			.unwrap();
		gone.channel_id = world.secret;
		assert_eq!(messages[1].condition.eval(&row(&gone)), Tri::Unknown);
		gone.deleted_at = Some(leptoskit_db::now());
		assert_eq!(messages[1].condition.eval(&row(&gone)), Tri::False);

		// Each message's author, through its link column: one row.
		let people = about::<Person>(&reads);
		assert_eq!(people.len(), 1);
		assert_eq!(people[0].limit, Some(1));
		assert_eq!(cards[0].latest[0].author.name, "Ada");

		// Each message's labels, through the link table.
		assert_eq!(about::<MessageLabel>(&reads).len(), 1);
		assert_eq!(about::<Label>(&reads).len(), 1);
		// The channels' policy and each message level's policy count
		// memberships.
		assert_eq!(about::<Membership>(&reads).len(), 3);

		// Without a tracked connection, a shape reads the same.
		let untracked = world
			.db
			.with_context(Context::new().with(Viewer(world.ada)));
		let card = Channel::query()
			.shape::<ChannelCard>()
			.one(&untracked)
			.await
			.unwrap();
		assert_eq!(card.name, "general");
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn views_record_the_tables_they_declare() {
	on_both(async |world: World| {
		let tracked = world.tracked(world.grace);
		let totals = ChannelTotal::query().all(&tracked).await.unwrap();
		assert_eq!(totals.len(), 2);
		let reads = tracked.into_reads();
		assert_eq!(reads.len(), 2);
		assert_eq!(reads[0].table, ChannelTotal::table_name());
		assert_eq!(reads[0].keys, None);
		assert_eq!(reads[1].table, Message::table_name());
		assert_eq!(reads[1].kind, ReadKind::Table);

		// A view that doesn't say what it reads can't be tracked, but reads
		// as ever otherwise.
		let tracked = world.tracked(world.grace);
		let refused = ChannelTally::query().all(&tracked).await.unwrap_err();
		assert!(matches!(
			refused,
			DbError::UndeclaredRead { table: Some(_) }
		));
		assert!(refused.to_string().contains("channel_tallies"));
		assert_eq!(tracked.reads(), Vec::<ReadEntry>::new());
		assert_eq!(ChannelTally::query().count(&world.db).await.unwrap(), 4);

		// A materialized view holds its own rows until refreshed.
		ChannelSnapshot::refresh(&world.db).await.unwrap();
		let tracked = world.tracked(world.grace);
		assert_eq!(ChannelSnapshot::query().count(&tracked).await.unwrap(), 2);
		let reads = tracked.into_reads();
		assert_eq!(reads.len(), 1);
		assert_eq!(reads[0].table, ChannelSnapshot::table_name());
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_sql_declares_the_tables_it_reads() {
	on_both(async |world: World| {
		let sql = "SELECT count(*) FROM messages JOIN people ON people.id = messages.author_id";
		let tracked = world.tracked(world.ada);

		let refused = tracked
			.fetch(Statement::new(sql, Vec::new()))
			.await
			.unwrap_err();
		assert!(matches!(refused, DbError::UndeclaredRead { table: None }));
		assert!(refused.to_string().contains("Statement::reads"));

		let rows = Statement::new(sql, Vec::new())
			.reads::<Message>()
			.reads::<Person>()
			.fetch(&tracked)
			.await
			.unwrap();
		assert_eq!(rows, [[Value::Int(4)]]);
		let reads = tracked.into_reads();
		assert_eq!(
			reads
				.iter()
				.map(|entry| entry.table.clone())
				.collect::<Vec<_>>(),
			[Message::table_name(), Person::table_name()]
		);
		assert!(reads.iter().all(|entry| entry.kind == ReadKind::Table));
		assert!(
			reads
				.iter()
				.all(|entry| entry.condition == db::Expr::always())
		);

		// Untracked, the declaration changes nothing.
		let rows = Statement::new(sql, Vec::new())
			.reads::<Message>()
			.fetch(&world.db)
			.await
			.unwrap();
		assert_eq!(rows, [[Value::Int(4)]]);
		let rows = Read::fetch(&&world.db, Statement::new(sql, Vec::new()))
			.await
			.unwrap();
		assert_eq!(rows, [[Value::Int(4)]]);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn searches_record_a_match_only_the_database_decides() {
	on_both(async |world: World| {
		let tracked = world.tracked(world.ada);
		let hits = Message::query()
			.search("hello")
			.filter(|message| message.channel_id.eq(world.general))
			.order_by(|message| message.created_at.asc())
			.limit(5)
			.all(&tracked)
			.await
			.unwrap();
		assert_eq!(hits.len(), 1);
		assert_eq!(
			Message::query()
				.search("again")
				.count(&tracked)
				.await
				.unwrap(),
			1
		);
		let cards: Vec<MessageCard> = Message::query()
			.search("hello")
			.shape()
			.all(&tracked)
			.await
			.unwrap();
		assert_eq!(cards.len(), 1);
		// A search with no words reads nothing, so nothing it reads can
		// change.
		let before = tracked.reads().len();
		assert!(
			Message::query()
				.search("  ")
				.all(&tracked)
				.await
				.unwrap()
				.is_empty()
		);
		assert_eq!(tracked.reads().len(), before);

		let reads = tracked.into_reads();
		let messages = about::<Message>(&reads);
		assert_eq!(
			messages.iter().map(|entry| entry.kind).collect::<Vec<_>>(),
			[ReadKind::Rows, ReadKind::Count, ReadKind::Rows]
		);
		// The rank orders the rows before any column does.
		assert_eq!(messages[0].order, Vec::new());
		assert_eq!(messages[0].limit, Some(5));
		assert_eq!(messages[0].keys, Some(keys(&[world.hello])));
		assert_eq!(messages[2].keys, None);

		// Whether a message matches the words only the database can say, but
		// one in another channel, or deleted, certainly doesn't.
		let mut message = Message::get(&world.db.system("test"), world.hello)
			.await
			.unwrap();
		assert_eq!(messages[0].condition.eval(&row(&message)), Tri::Unknown);
		message.channel_id = world.secret;
		assert_eq!(messages[0].condition.eval(&row(&message)), Tri::False);
		message.channel_id = world.general;
		message.deleted_at = Some(leptoskit_db::now());
		assert_eq!(messages[1].condition.eval(&row(&message)), Tri::False);

		// The shaped search reads each card's author and labels too.
		assert_eq!(about::<Person>(&reads).len(), 1);
		assert_eq!(about::<MessageLabel>(&reads).len(), 1);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_transaction_can_be_tracked() {
	on_both(async |world: World| {
		let request = world
			.db
			.with_context(Context::new().with(Viewer(world.ada)));
		let tx = request.begin().await.unwrap();
		let tracked = Tracked::new(&tx);
		assert_eq!(tracked.dialect(), world.db.dialect());
		let channels = Channel::query().all(&tracked).await.unwrap();
		assert_eq!(channels.len(), 1);
		let reads = tracked.into_reads();
		tx.commit().await.unwrap();

		// The channel policy's membership count.
		assert_eq!(reads.len(), 2);
		assert_eq!(reads[1].table, Membership::table_name());
		let grace_in_secret = Membership::query()
			.filter(|member| member.person_id.eq(world.grace))
			.one(&world.db.system("test"))
			.await
			.unwrap();
		assert_eq!(reads[1].condition.eval(&row(&grace_in_secret)), Tri::False);
	})
	.await;
}

#[test]
fn public_types_carry_their_rows_key() {
	let id = Id::<Channel>::new();
	let channel = PublicChannel {
		id,
		name: "general".into(),
		private: false,
	};
	assert_eq!(channel.id(), id);
}
