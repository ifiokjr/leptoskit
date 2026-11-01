//! Live changes against both databases: every write path's changes reach
//! the change log and subscribers once it commits, nothing a rollback undid,
//! the rows the database's own cascades change too, and each subscriber
//! hears only of rows its policy lets it read, before or after the change.
//! A subscriber that falls behind, or whose server lost its connection,
//! catches up from the log.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use std::time::Duration;

use leptoskit_db::AuditEntry;
use leptoskit_db::AuditOp;
use leptoskit_db::AuditReader;
use leptoskit_db::Connection;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::LiveCapture;
use leptoskit_db::Timestamp;
use leptoskit_db::live::Capture;
use leptoskit_db::live::CaptureSettings;
use leptoskit_db::live::ChangedRows;
use leptoskit_db::live::Feed;
use leptoskit_db::live::Invalidation;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest::test_runner::TestCaseError;
use proptest::test_runner::TestRunner;

/// Who the request is.
struct Viewer(i64);

/// A note: anyone reads a public one, and its owner reads it either way.
#[table(name = "live_notes", live)]
#[policy(select, |note, cx| note.public.eq(true) | cx.global(|viewer: &Viewer| note.owner.eq(viewer.0)))]
#[policy(insert, update, delete, |_, _| Expr::always())]
pub struct Note {
	pub id: Id<Self>,
	pub owner: i64,
	pub public: bool,
	pub body: String,
	#[db(secret)]
	pub token: String,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// A pin on a note, gone with it: the database deletes it.
#[table(name = "live_pins", live, audit)]
pub struct Pin {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub note_id: Id<Note>,
}

/// A reply to a pin, gone with the pin: a cascade two levels deep.
#[table(name = "live_replies", live)]
pub struct Reply {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub pin_id: Id<Pin>,
}

/// A mention of a note, which outlives it with its link emptied.
#[table(name = "live_mentions", live, audit)]
pub struct Mention {
	pub id: Id<Self>,
	pub note_id: Option<Id<Note>>,
}

/// A marker everyone sees, written after each step of a test so that
/// "heard nothing" is "heard the marker next".
#[table(name = "live_markers", live)]
pub struct Marker {
	pub id: Id<Self>,
}

/// Changes nobody subscribes to: not live.
#[table(name = "quiet_notes")]
pub struct Quiet {
	pub id: Id<Self>,
}

async fn create_schema(db: &Db) {
	create_schema_capturing(db, LiveCapture::Triggers).await;
}

/// The tables, with writes to live ones captured by `capture`.
async fn create_schema_capturing(db: &Db, capture: LiveCapture) {
	let dialect = db.dialect();
	let schema =
		leptoskit_db::with_live_capture(leptoskit_db::schema(dialect).unwrap(), capture).unwrap();
	let empty = Schema::empty(dialect);
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
	let migration = leptoskit_db_schema::render(&plan, &empty, &schema);
	db.run_script(&migration.statements.join(";\n"))
		.await
		.unwrap();
}

/// Runs `check` on SQLite, then on Postgres capturing with triggers, then on
/// Postgres capturing by logical replication, which must behave the same to
/// every reader.
async fn on_both<F, Fut>(check: F)
where
	F: Fn(Db, Option<String>) -> Fut,
	Fut: Future<Output = ()>,
{
	let db = Db::connect("sqlite::memory:").await.unwrap();
	create_schema(&db).await;
	check(db, None).await;

	if let Some(cluster) = support::postgres::PostgresCluster::start() {
		let url = cluster.database();
		let db = Db::connect(&url).await.unwrap();
		create_schema(&db).await;
		check(db, Some(url)).await;

		let url = cluster.database();
		let db = Db::connect(&url).await.unwrap();
		create_schema_capturing(&db, LiveCapture::Replication).await;
		let capture = start_capture(&db).await;
		check(db, Some(url)).await;
		capture.shutdown().await;
	}
}

/// Replication capture, once it's capturing: its slot exists, and the
/// marker that says whatever came before it may have been missed is behind
/// every feed a test makes from here.
async fn start_capture(db: &Db) -> Capture {
	let capture = db.capture_changes_with(fast_capture());

	tokio::time::timeout(Duration::from_secs(20), async {
		while !capture.stats().capturing {
			tokio::time::sleep(Duration::from_millis(10)).await;
		}
	})
	.await
	.expect("capture starts");

	capture
}

/// Replication capture that reads the slot often, so tests wait little.
fn fast_capture() -> CaptureSettings {
	CaptureSettings::default()
		.poll(Duration::from_millis(10))
		.standby(Duration::from_millis(50))
}

fn as_viewer(db: &Db, viewer: i64) -> Db {
	db.with_context(Context::new().with(Viewer(viewer)))
}

/// The next change, or a failure after a few seconds.
async fn next(feed: &mut Feed) -> Invalidation {
	tokio::time::timeout(Duration::from_secs(10), feed.next())
		.await
		.expect("a change arrives")
		.unwrap()
}

fn rows(table: &str, keys: impl IntoIterator<Item = impl ToString>) -> ChangedRows {
	ChangedRows {
		table: table.into(),
		keys: keys.into_iter().map(|key| vec![key.to_string()]).collect(),
	}
}

fn changed(changes: &[ChangedRows]) -> Invalidation {
	Invalidation::Rows(changes.to_vec())
}

async fn note(db: &Db, owner: i64, public: bool) -> Note {
	NewNote {
		owner,
		public,
		body: "hello".into(),
		token: "secret".into(),
	}
	.insert(db)
	.await
	.unwrap()
}

async fn note_in(connection: &impl Connection, owner: i64) -> Result<Note, DbError> {
	NewNote {
		owner,
		public: true,
		body: "b".into(),
		token: "t".into(),
	}
	.insert(connection)
	.await
}

/// Writes a marker, and checks it is the next thing `feed` hears.
async fn hears_nothing_else(db: &Db, feed: &mut Feed) {
	none_hear_anything_else(db, &mut [feed]).await;
}

/// Writes a marker, and checks it is the next thing each feed hears.
async fn none_hear_anything_else(db: &Db, feeds: &mut [&mut Feed]) {
	let marker = NewMarker {}.insert(db).await.unwrap();

	for feed in feeds {
		assert_eq!(
			next(feed).await,
			changed(&[rows("live_markers", [marker.id.to_string()])])
		);
	}
}

#[tokio::test]
async fn every_write_path_publishes_what_it_changed() {
	on_both(async |db, _| {
		let system = db.system("a live test");
		let mut feed = system.changes().await.unwrap();

		let mut first = note(&system, 1, true).await;
		let id = first.id.to_string();
		assert_eq!(next(&mut feed).await, changed(&[rows("live_notes", [&id])]));

		first.body = "changed".into();
		first.save(&system).await.unwrap();
		assert_eq!(next(&mut feed).await, changed(&[rows("live_notes", [&id])]));

		Note::update()
			.filter(|note| note.owner.eq(1))
			.set(|note| note.body, "bulk".to_string())
			.run(&system)
			.await
			.unwrap();
		assert_eq!(next(&mut feed).await, changed(&[rows("live_notes", [&id])]));

		first.delete(&system).await.unwrap();
		assert_eq!(next(&mut feed).await, changed(&[rows("live_notes", [&id])]));

		first.destroy(&system).await.unwrap();
		assert_eq!(next(&mut feed).await, changed(&[rows("live_notes", [id])]));

		// One transaction is one change set, published once it commits.
		let (left, right) = system
			.transaction(async |tx| {
				let left = note_in(tx, 2).await?;
				let right = note_in(tx, 2).await?;
				Ok::<_, DbError>((left, right))
			})
			.await
			.unwrap();
		let mut both = vec![left.id.to_string(), right.id.to_string()];
		both.sort();
		let Invalidation::Rows(heard) = next(&mut feed).await else {
			panic!("expected rows");
		};
		let mut keys: Vec<String> = heard[0].keys.iter().map(|key| key[0].clone()).collect();
		keys.sort();
		assert_eq!((heard.len(), keys), (1, both));

		system
			.transaction(async |tx| {
				NewQuiet {}.insert(tx).await?;
				note_in(tx, 3).await?.destroy(tx).await?;
				Note::query()
					.filter(|note| note.owner.eq(2))
					.destroy(tx)
					.await?;
				Ok::<_, DbError>(())
			})
			.await
			.unwrap();
		// The new note, inserted and removed, and both of owner 2's, in one
		// set: the quiet table publishes nothing.
		let Invalidation::Rows(together) = next(&mut feed).await else {
			panic!("expected rows");
		};
		assert_eq!(together.len(), 1);
		assert_eq!(together[0].table, "live_notes");
		assert_eq!(together[0].keys.len(), 3);

		hears_nothing_else(&system, &mut feed).await;
	})
	.await;
}

#[tokio::test]
async fn rolled_back_changes_are_never_published() {
	on_both(async |db, _| {
		let system = db.system("a live test");
		let mut feed = system.changes().await.unwrap();

		let failed: Result<(), DbError> = system
			.transaction(async |tx| {
				NewNote {
					owner: 1,
					public: true,
					body: "never".into(),
					token: "t".into(),
				}
				.insert(tx)
				.await?;
				Err(DbError::NotFound {
					table: "on purpose".into(),
				})
			})
			.await;
		assert!(failed.is_err());

		let tx = system.begin().await.unwrap();
		note_in(&tx, 1).await.unwrap();
		NewMarker {}.insert(&tx).await.unwrap();
		tx.rollback().await.unwrap();

		// Dropped part-way, a transaction rolls back too.
		let tx = system.begin().await.unwrap();
		note_in(&tx, 1).await.unwrap();
		drop(tx);

		hears_nothing_else(&system, &mut feed).await;
	})
	.await;
}

#[tokio::test]
async fn subscribers_hear_only_of_rows_they_could_read_before_or_after() {
	on_both(async |db, _| {
		let system = db.system("a live test");
		let mut owner = as_viewer(&db, 1).changes().await.unwrap();
		let mut stranger = as_viewer(&db, 2).changes().await.unwrap();
		let mut signed_out = db.changes().await.unwrap();

		// Private: only its owner hears of it.
		let mut private = note(&system, 1, false).await;
		let key = private.id.to_string();
		assert_eq!(
			next(&mut owner).await,
			changed(&[rows("live_notes", [&key])])
		);
		none_hear_anything_else(&system, &mut [&mut owner, &mut stranger, &mut signed_out]).await;

		// Made public: everyone could read it after.
		private.public = true;
		private.save(&system).await.unwrap();

		for feed in [&mut owner, &mut stranger, &mut signed_out] {
			assert_eq!(next(feed).await, changed(&[rows("live_notes", [&key])]));
		}

		// Made private again: everyone could read it before.
		private.public = false;
		private.save(&system).await.unwrap();

		for feed in [&mut owner, &mut stranger, &mut signed_out] {
			assert_eq!(next(feed).await, changed(&[rows("live_notes", [&key])]));
		}

		// Changed while private, and deleted: only its owner hears.
		private.body = "edited".into();
		private.save(&system).await.unwrap();
		private.destroy(&system).await.unwrap();
		assert_eq!(
			next(&mut owner).await,
			changed(&[rows("live_notes", [&key])])
		);
		assert_eq!(
			next(&mut owner).await,
			changed(&[rows("live_notes", [key])])
		);
		none_hear_anything_else(&system, &mut [&mut owner, &mut stranger, &mut signed_out]).await;
	})
	.await;
}

#[tokio::test]
async fn policies_never_see_secret_columns() {
	/// Readable only by its token, which is secret.
	#[table(name = "live_vaults", live)]
	#[policy(select, |vault, _| vault.token.eq("open sesame"))]
	pub struct Vault {
		pub id: Id<Self>,
		#[db(secret)]
		pub token: String,
	}

	on_both(async |db, _| {
		let system = db.system("a live test");
		let mut anyone = db.changes().await.unwrap();
		let vault = NewVault {
			token: "open sesame".into(),
		}
		.insert(&system)
		.await
		.unwrap();
		// The policy matches the stored row…
		assert_eq!(Vault::query().count(&db).await.unwrap(), 1);
		// …but a change carries no secret, so it can't match one: a policy on
		// a secret column fails closed.
		hears_nothing_else(&system, &mut anyone).await;
		drop(vault);
	})
	.await;
}

#[tokio::test]
async fn the_database_s_own_cascades_are_published_and_audited() {
	on_both(async |db, _| {
		let system = db.system("a live test");
		let reader = system.with_context(Context::new().with(AuditReader::everything()));
		let parent = note(&system, 1, true).await;
		let pin = NewPin { note_id: parent.id }.insert(&system).await.unwrap();
		let reply = NewReply { pin_id: pin.id }.insert(&system).await.unwrap();
		let mention = NewMention {
			note_id: Some(parent.id),
		}
		.insert(&system)
		.await
		.unwrap();
		// Replication capture records the writes above a moment after they
		// commit; the feed starts after them.
		system.live_caught_up().await.unwrap();
		let mut feed = system.changes().await.unwrap();

		let parent_id = parent.id;
		parent.destroy(&system).await.unwrap();

		let Invalidation::Rows(mut sets) = next(&mut feed).await else {
			panic!("expected rows");
		};
		sets.sort_by(|left, right| left.table.cmp(&right.table));
		assert_eq!(
			sets,
			[
				rows("live_mentions", [mention.id.to_string()]),
				rows("live_notes", [parent_id.to_string()]),
				rows("live_pins", [pin.id.to_string()]),
				rows("live_replies", [reply.id.to_string()]),
			]
		);
		assert_eq!(Reply::query().count(&system).await.unwrap(), 0);
		assert_eq!(Mention::query().one(&system).await.unwrap().note_id, None);

		let pins = AuditEntry::for_table::<Pin>().all(&reader).await.unwrap();
		assert_eq!(
			pins.iter().map(|entry| entry.op).collect::<Vec<_>>(),
			[AuditOp::Insert, AuditOp::Delete]
		);
		assert!(pins[1].after.is_none());
		let mentions = AuditEntry::for_table::<Mention>()
			.all(&reader)
			.await
			.unwrap();
		assert_eq!(mentions.len(), 2);
		assert_eq!(mentions[1].op, AuditOp::Update);
		assert_eq!(
			mentions[1].after.as_ref().unwrap().0["note_id"],
			serde_json::Value::Null
		);
		assert_ne!(
			mentions[1].before.as_ref().unwrap().0["note_id"],
			serde_json::Value::Null
		);

		// A bulk destroy follows the cascades too.
		let other = note(&system, 2, true).await;
		let pin = NewPin { note_id: other.id }.insert(&system).await.unwrap();
		next(&mut feed).await;
		next(&mut feed).await;
		Note::query()
			.filter(|note| note.owner.eq(2))
			.destroy(&system)
			.await
			.unwrap();
		let Invalidation::Rows(sets) = next(&mut feed).await else {
			panic!("expected rows");
		};
		assert!(sets.contains(&rows("live_pins", [pin.id.to_string()])));
		assert_eq!(
			AuditEntry::for_table::<Pin>().count(&reader).await.unwrap(),
			4
		);

		// A refused delete records and publishes nothing.
		hears_nothing_else(&system, &mut feed).await;
	})
	.await;
}

#[tokio::test]
async fn changes_reach_every_server_sharing_a_postgres_database() {
	on_both(async |db, url| {
		let Some(url) = url else {
			return;
		};
		let elsewhere = Db::connect(&url).await.unwrap();
		let mut feed = elsewhere.system("another server").changes().await.unwrap();

		// Large enough to take several notifications.
		let body = "é".repeat(20_000);
		let written = NewNote {
			owner: 1,
			public: true,
			body,
			token: "t".into(),
		}
		.insert(&db.system("this server"))
		.await
		.unwrap();
		assert_eq!(
			next(&mut feed).await,
			changed(&[rows("live_notes", [written.id.to_string()])])
		);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lost_listening_connection_reconnects_and_catches_up() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};

	for capture in [LiveCapture::Triggers, LiveCapture::Replication] {
		reconnects_and_catches_up(&cluster.database(), capture).await;
	}
}

async fn reconnects_and_catches_up(url: &str, capture: LiveCapture) {
	let url = url.to_string();
	let db = Db::connect(&url).await.unwrap();
	create_schema_capturing(&db, capture).await;
	let _capture = match capture {
		LiveCapture::Triggers => None,
		LiveCapture::Replication => Some(start_capture(&db).await),
	};
	let system = db.system("a live test");
	let mut feed = system.changes().await.unwrap();

	// What an operator restarting the database would do to the connection.
	system
		.run_script(
			"SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE query LIKE 'LISTEN%' \
			 AND pid <> pg_backend_pid()",
		)
		.await
		.unwrap();

	// Written while nothing listens: their notifications are lost, but the
	// log keeps them, and the reconnection reads it.
	let mut written = Vec::new();

	for _ in 0..3 {
		written.push(NewMarker {}.insert(&system).await.unwrap().id.to_string());
	}

	for marker in &written {
		assert_eq!(
			next(&mut feed).await,
			changed(&[rows("live_markers", [marker])]),
			"heard in order, with nothing to refetch"
		);
	}

	// Listening again, later changes arrive as before.
	hears_nothing_else(&system, &mut feed).await;

	// A database that isn't there fails the first subscription.
	// Port 1 has no server.
	let port = url.split(':').nth(2).unwrap().split('/').next().unwrap();
	let closed = url.replace(&format!(":{port}/"), ":1/");
	let unreachable = Db::connect(&closed).await.unwrap();
	assert!(unreachable.changes().await.is_err());
}

#[tokio::test]
async fn a_subscriber_that_falls_behind_catches_up_from_the_log() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	create_schema(&db).await;
	let system = db.system("a live test");
	let mut feed = system.changes().await.unwrap();
	let mut written = Vec::new();

	for _ in 0..1100 {
		written.push(NewMarker {}.insert(&system).await.unwrap().id.to_string());
	}

	for marker in &written {
		assert_eq!(
			next(&mut feed).await,
			changed(&[rows("live_markers", [marker])])
		);
	}

	assert!(format!("{feed:?}").starts_with("Feed"));
}

#[tokio::test]
async fn a_feed_resumes_from_a_version_and_learns_when_it_was_pruned() {
	on_both(async |db, _| {
		let system = db.system("a live test");
		let mut feed = system.changes().await.unwrap();
		let first = NewMarker {}.insert(&system).await.unwrap();
		let heard = tokio::time::timeout(Duration::from_secs(10), feed.next_versioned())
			.await
			.unwrap()
			.unwrap();
		assert_eq!(
			heard.invalidation,
			changed(&[rows("live_markers", [first.id.to_string()])])
		);
		assert_eq!(heard.version, feed.version());
		let after_first = feed.version();

		// A client that went away after the first comes back for the rest.
		let second = NewMarker {}.insert(&system).await.unwrap();
		let third = NewMarker {}.insert(&system).await.unwrap();
		let mut resumed = system.changes_since(after_first).await.unwrap();

		for marker in [&second, &third] {
			assert_eq!(
				next(&mut resumed).await,
				changed(&[rows("live_markers", [marker.id.to_string()])])
			);
		}

		// Pruned past where a feed is: it can't know what it missed.
		let newest = system.live_version().await.unwrap();
		assert_eq!(newest, resumed.version());
		assert!(system.prune_changes(Duration::ZERO).await.unwrap() > 0);
		let mut behind = system.changes_since(after_first).await.unwrap();
		assert_eq!(next(&mut behind).await, Invalidation::Everything);
		assert_eq!(behind.version(), newest);

		assert_eq!(next(&mut feed).await, Invalidation::Everything);

		// A feed that was up to date misses nothing, and all go on hearing.
		none_hear_anything_else(&system, &mut [&mut resumed, &mut behind, &mut feed]).await;
	})
	.await;
}

// ---- the property ----------------------------------------------------------
/// One step of a random history, each by the system.
#[derive(Clone, Debug)]
enum Step {
	/// A new note by an owner, public or not.
	Insert { owner: i64, public: bool },
	/// A note made public or private, and its body changed.
	Set { note: usize, public: bool },
	/// A note given to another owner.
	Give { note: usize, owner: i64 },
	/// A note soft-deleted.
	Delete { note: usize },
	/// A note removed for good.
	Destroy { note: usize },
	/// Every note of an owner made public or private at once.
	SetAll { owner: i64, public: bool },
}

fn steps() -> impl Strategy<Value = Vec<Step>> {
	let owner = 1..4_i64;
	let note = 0..6_usize;
	proptest::collection::vec(
		prop_oneof![
			3 => (owner.clone(), any::<bool>()).prop_map(|(owner, public)| Step::Insert { owner, public }),
			3 => (note.clone(), any::<bool>()).prop_map(|(note, public)| Step::Set { note, public }),
			2 => (note.clone(), owner.clone()).prop_map(|(note, owner)| Step::Give { note, owner }),
			1 => note.clone().prop_map(|note| Step::Delete { note }),
			1 => note.prop_map(|note| Step::Destroy { note }),
			1 => (owner, any::<bool>()).prop_map(|(owner, public)| Step::SetAll { owner, public }),
		],
		1..10,
	)
}

/// A note as the model holds it.
#[derive(Clone, Copy, Debug)]
struct Modelled {
	owner: i64,
	public: bool,
}

/// Whether `viewer` may read `note`, as the policy says.
fn readable(note: Modelled, viewer: i64) -> bool {
	note.public || note.owner == viewer
}

/// What each step changes, as (key, before, after), and runs it.
async fn apply(
	db: &Db,
	notes: &mut Vec<(Note, Option<Modelled>)>,
	step: &Step,
) -> Vec<(String, Option<Modelled>, Option<Modelled>)> {
	let system = db.system("a live property");
	let alive = |notes: &Vec<(Note, Option<Modelled>)>, index: usize| {
		let live: Vec<usize> = (0..notes.len()).filter(|&i| notes[i].1.is_some()).collect();
		(!live.is_empty()).then(|| live[index % live.len()])
	};

	match *step {
		Step::Insert { owner, public } => {
			let row = note(&system, owner, public).await;
			let model = Modelled { owner, public };
			let key = row.id.to_string();
			notes.push((row, Some(model)));
			vec![(key, None, Some(model))]
		}
		Step::Set { note, public } => {
			let Some(index) = alive(notes, note) else {
				return Vec::new();
			};
			let (row, model) = &mut notes[index];
			let before = *model;
			row.public = public;
			row.body.push('!');
			row.save(&system).await.unwrap();
			model.as_mut().unwrap().public = public;
			vec![(row.id.to_string(), before, *model)]
		}
		Step::Give { note, owner } => {
			let Some(index) = alive(notes, note) else {
				return Vec::new();
			};
			let (row, model) = &mut notes[index];
			let before = *model;
			row.owner = owner;
			row.save(&system).await.unwrap();
			model.as_mut().unwrap().owner = owner;
			vec![(row.id.to_string(), before, *model)]
		}
		Step::Delete { note } => {
			let Some(index) = alive(notes, note) else {
				return Vec::new();
			};
			let (row, model) = &mut notes[index];
			let before = *model;
			row.delete(&system).await.unwrap();
			// A soft-deleted row is still one its readers may ask for.
			*model = None;
			vec![(row.id.to_string(), before, before)]
		}
		Step::Destroy { note } => {
			let Some(index) = alive(notes, note) else {
				return Vec::new();
			};
			let (row, model) = &mut notes[index];
			let before = model.take();
			let id = row.id;
			Note::query()
				.filter(|note| note.id.eq(id))
				.with_deleted()
				.destroy(&system)
				.await
				.unwrap();
			vec![(id.to_string(), before, None)]
		}
		Step::SetAll { owner, public } => {
			Note::update()
				.filter(|note| note.owner.eq(owner))
				.set(|note| note.public, public)
				.run(&system)
				.await
				.unwrap();
			let mut changes = Vec::new();

			for (row, model) in notes.iter_mut() {
				if let Some(model) = model.as_mut().filter(|model| model.owner == owner) {
					let before = *model;
					model.public = public;
					row.public = public;
					changes.push((row.id.to_string(), Some(before), Some(*model)));
				}
			}

			changes
		}
	}
}

async fn agrees_with_the_model(db: &Db, steps: Vec<Step>) -> Result<(), TestCaseError> {
	// Viewers 1 to 3 own notes; 4 owns none.
	let viewers = [1, 2, 3, 4];
	let mut feeds = Vec::new();

	for viewer in viewers {
		feeds.push(as_viewer(db, viewer).changes().await.unwrap());
	}

	// Forget the last case's notes, and whatever that tells each feed.
	let system = db.system("reset");
	Note::query().destroy(&system).await.unwrap();
	let marker = NewMarker {}.insert(&system).await.unwrap();
	let reset_done = changed(&[rows("live_markers", [marker.id.to_string()])]);

	for feed in &mut feeds {
		while next(feed).await != reset_done {}
	}

	let mut notes = Vec::new();

	for step in &steps {
		let changes = apply(db, &mut notes, step).await;
		let marker = NewMarker {}.insert(&db.system("a marker")).await.unwrap();

		for (viewer, feed) in viewers.iter().zip(&mut feeds) {
			let mut expected: Vec<String> = changes
				.iter()
				.filter(|(_, before, after)| {
					[before, after]
						.into_iter()
						.flatten()
						.any(|note| readable(*note, *viewer))
				})
				.map(|(key, ..)| key.clone())
				.collect();
			expected.sort();

			if !expected.is_empty() {
				let Invalidation::Rows(heard) = next(feed).await else {
					return Err(TestCaseError::fail("expected rows, heard everything"));
				};
				prop_assert_eq!(heard.len(), 1);
				prop_assert_eq!(&heard[0].table, "live_notes");
				let mut keys: Vec<String> =
					heard[0].keys.iter().map(|key| key[0].clone()).collect();
				keys.sort();

				prop_assert_eq!(keys, expected, "viewer {} after {:?}", viewer, step);
			}

			prop_assert_eq!(
				next(feed).await,
				changed(&[rows("live_markers", [marker.id.to_string()])]),
				"viewer {} heard more after {:?}",
				viewer,
				step
			);
		}
	}

	Ok(())
}

fn run_property(db: &Db, runtime: &tokio::runtime::Runtime, cases: u32) {
	let mut runner = TestRunner::new(Config {
		cases,
		failure_persistence: None,
		..Config::default()
	});
	let outcome = runner.run(&steps(), |steps| {
		runtime.block_on(agrees_with_the_model(db, steps))
	});

	if let Err(error) = outcome {
		panic!("{error}");
	}
}

#[test]
fn what_each_viewer_hears_agrees_with_the_model_on_sqlite() {
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let db = runtime.block_on(async {
		let db = Db::connect("sqlite::memory:").await.unwrap();
		create_schema(&db).await;
		db
	});
	run_property(&db, &runtime, 96);
}

#[test]
fn what_each_viewer_hears_agrees_with_the_model_on_postgres() {
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

#[test]
fn what_each_viewer_hears_agrees_with_the_model_with_replication_capture() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let (db, capture) = runtime.block_on(async {
		let db = Db::connect(&cluster.database()).await.unwrap();
		create_schema_capturing(&db, LiveCapture::Replication).await;
		let capture = start_capture(&db).await;
		(db, capture)
	});
	run_property(&db, &runtime, 24);
	runtime.block_on(capture.shutdown());
}
