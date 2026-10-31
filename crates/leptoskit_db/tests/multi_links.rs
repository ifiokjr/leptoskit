//! Many-to-many links, `#[db::link]`, against both databases: adding,
//! reading, and removing links both ways, link properties, links to the same
//! table, `exclusive`, and policies, with a property test comparing what
//! each request sees through links with a plain Rust model.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use leptoskit_db as db;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Link;
use leptoskit_db::Table;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db_schema::ReferentialAction;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use leptoskit_db_schema::TableName;
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest::test_runner::TestCaseError;
use proptest::test_runner::TestRunner;

/// The signed-in writer: the global the policies read.
struct Viewer(Id<Writer>);

/// Someone who writes notes. No policies.
#[table(name = "writers")]
pub struct Writer {
	pub id: Id<Self>,
	pub name: String,
}

/// A note: published ones are for everyone, drafts for their writer, and
/// only the writer changes one.
#[table(name = "notes")]
#[policy(select, |note, cx| note.published.eq(true) | cx.global(|viewer: &Viewer| note.writer_id.eq(viewer.0)))]
#[policy(insert, update, delete, |note, cx| cx.global(|viewer: &Viewer| note.writer_id.eq(viewer.0)))]
pub struct Note {
	pub id: Id<Self>,
	pub writer_id: Id<Writer>,
	pub title: String,
	pub published: bool,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// A label for notes. No policies, but labels can be soft-deleted.
#[table(name = "labels")]
pub struct Label {
	pub id: Id<Self>,
	pub name: String,
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// A label on a note. Anyone sees a link unless it's private, which only
/// the note's writer sees; only the writer adds, changes, or removes one.
#[db::link(name = "note_labels", from = Note as labels, to = Label as notes)]
#[policy(select, |link, cx| link.private.eq(false) | writes_the_note(link, cx))]
#[policy(insert, update, delete, |link, cx| writes_the_note(link, cx))]
pub struct NoteLabel {
	pub position: i16,
	pub private: bool,
	#[db(created_at)]
	pub added_at: Timestamp,
}

fn writes_the_note(link: &NoteLabelColumns, cx: &Context) -> Expr {
	cx.global(|viewer: &Viewer| {
		cx.exists(
			Note::query()
				.with_deleted()
				.filter(|note| note.id.eq(link.note_id) & note.writer_id.eq(viewer.0)),
		)
	})
}

/// One writer following another: a link from a table to itself, with
/// nothing on it but its ends.
#[db::link(
	name = "follows",
	from = Writer as following,
	to = Writer as followers,
	from_column = "follower_id",
	to_column = "followed_id"
)]
pub struct Follow {}

/// The one note a label is pinned to: `exclusive` lets each label be linked
/// from one note only.
#[db::link(name = "pins", from = Note as pins, to = Label as pinned_on, exclusive)]
pub struct Pin {}

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

async fn writer(db: &Db, name: &str) -> Writer {
	NewWriter { name: name.into() }.insert(db).await.unwrap()
}

async fn note(db: &Db, writer: &Writer, title: &str, published: bool) -> Note {
	NewNote {
		writer_id: writer.id,
		title: title.into(),
		published,
	}
	.insert(&as_writer(db, writer))
	.await
	.unwrap()
}

async fn label(db: &Db, name: &str) -> Label {
	NewLabel { name: name.into() }.insert(db).await.unwrap()
}

fn as_writer(db: &Db, writer: &Writer) -> Db {
	db.with_context(Context::new().with(Viewer(writer.id)))
}

fn public(position: i16) -> NoteLabelProperties {
	NoteLabelProperties {
		position,
		private: false,
	}
}

#[test]
fn a_link_is_a_table_keyed_by_its_two_ends() {
	let table = NoteLabel::definition();
	assert_eq!(table.name, TableName::new("note_labels"));
	assert_eq!(table.primary_key, ["note_id", "label_id"]);
	assert_eq!(
		NoteLabel::COLUMNS,
		["note_id", "label_id", "position", "private", "added_at"]
	);
	assert_eq!((NoteLabel::FROM, NoteLabel::TO), ("note_id", "label_id"));

	// Deleting either end deletes its links.
	let actions: Vec<(&str, ReferentialAction)> = table
		.foreign_keys
		.iter()
		.map(|key| (key.columns[0].as_str(), key.on_delete))
		.collect();
	assert_eq!(
		actions,
		[
			("note_id", ReferentialAction::Cascade),
			("label_id", ReferentialAction::Cascade),
		]
	);

	// The key covers lookups from the first end; the other has an index.
	let indexes: Vec<(&[String], bool)> = table
		.indexes
		.iter()
		.map(|index| (index.columns.as_slice(), index.unique))
		.collect();
	assert_eq!(indexes, [(&["label_id".to_string()][..], false)]);

	// `exclusive` makes the far end unique instead.
	let pins = Pin::definition();
	assert_eq!(pins.indexes.len(), 1);
	assert!(pins.indexes[0].unique);
	assert_eq!(pins.indexes[0].columns, ["label_id"]);

	// A link to the same table names its columns.
	assert_eq!(
		Follow::definition().primary_key,
		["follower_id", "followed_id"]
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn links_are_added_read_and_removed_both_ways() {
	on_both(async |db| {
		let ada = writer(&db, "Ada").await;
		let as_ada = as_writer(&db, &ada);
		let engines = note(&db, &ada, "Engines", true).await;
		let looms = note(&db, &ada, "Looms", true).await;
		let maths = label(&db, "maths").await;
		let history = label(&db, "history").await;

		let link = engines
			.labels()
			.add(&as_ada, maths.id, public(2))
			.await
			.unwrap();
		assert_eq!((link.note_id, link.label_id), (engines.id, maths.id));
		assert_eq!(link.source_id(), engines.id);
		assert_eq!(link.target_id(), maths.id);
		engines
			.labels()
			.add(&as_ada, history.id, public(1))
			.await
			.unwrap();
		maths
			.notes()
			.add(&as_ada, looms.id, public(0))
			.await
			.unwrap();

		// Both directions, ordered by the link's own column.
		let labels = engines
			.labels()
			.order_by(|link| link.position.asc())
			.all(&db)
			.await
			.unwrap();
		let names: Vec<(&str, i16)> = labels
			.iter()
			.map(|(label, link)| (label.name.as_str(), link.position))
			.collect();
		assert_eq!(names, [("history", 1), ("maths", 2)]);
		assert_eq!(maths.notes().count(&db).await.unwrap(), 2);
		let mut noted = maths.notes().ids(&db).await.unwrap();
		noted.sort();
		let mut expected = vec![engines.id, looms.id];
		expected.sort();
		assert_eq!(noted, expected);
		assert!(engines.labels().contains(&db, maths.id).await.unwrap());
		assert!(!looms.labels().contains(&db, history.id).await.unwrap());

		// Filters on either side, and paging over links.
		let only_maths = engines
			.labels()
			.filter(|label| label.name.eq("maths"))
			.ids(&db)
			.await
			.unwrap();
		assert_eq!(only_maths, [maths.id]);
		let early = engines
			.labels()
			.filter_link(|link| link.position.lt(2))
			.ids(&db)
			.await
			.unwrap();
		assert_eq!(early, [history.id]);
		let second = engines
			.labels()
			.order_by(|link| link.position.asc())
			.offset(1)
			.limit(1)
			.link_rows(&db)
			.await
			.unwrap();
		assert_eq!(second.len(), 1);
		assert_eq!(second[0].label_id, maths.id);

		// Adding again replaces the link's properties; there's still one.
		let moved = engines
			.labels()
			.add(&as_ada, maths.id, public(9))
			.await
			.unwrap();
		assert_eq!(moved.position, 9);
		assert_eq!(moved.added_at, link.added_at);
		assert_eq!(engines.labels().count(&db).await.unwrap(), 2);

		assert!(engines.labels().remove(&as_ada, maths.id).await.unwrap());
		assert!(!engines.labels().remove(&as_ada, maths.id).await.unwrap());
		assert_eq!(engines.labels().ids(&db).await.unwrap(), [history.id]);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn links_to_hidden_or_deleted_rows_are_not_there() {
	on_both(async |db| {
		let ada = writer(&db, "Ada").await;
		let as_ada = as_writer(&db, &ada);
		let mut engines = note(&db, &ada, "Engines", true).await;
		let mut maths = label(&db, "maths").await;
		let draft = note(&db, &ada, "Draft", false).await;
		engines
			.labels()
			.add(&as_ada, maths.id, public(0))
			.await
			.unwrap();
		draft
			.labels()
			.add(&as_ada, maths.id, public(0))
			.await
			.unwrap();

		// The draft is Ada's alone, so only she sees it through the label.
		assert_eq!(maths.notes().count(&as_ada).await.unwrap(), 2);
		assert_eq!(maths.notes().ids(&db).await.unwrap(), [engines.id]);
		// Nor can anyone else follow links from it.
		let hidden = leptoskit_db::Links::<NoteLabel, _, _>::forward(draft.id);
		assert_eq!(hidden.count(&db).await.unwrap(), 0);

		// A soft-deleted end hides its links, which come back with it.
		maths.delete(&db).await.unwrap();
		assert!(engines.labels().all(&db).await.unwrap().is_empty());
		maths.deleted_at = None;
		maths.save(&db).await.unwrap();
		assert_eq!(engines.labels().count(&db).await.unwrap(), 1);
		engines.delete(&as_ada).await.unwrap();
		assert_eq!(maths.notes().count(&as_ada).await.unwrap(), 1);

		// Removing an end for good removes its links with it.
		let system = db.system("destroying a label");
		maths.destroy(&system).await.unwrap();
		assert_eq!(
			NoteLabel::query()
				.with_deleted()
				.count(&system)
				.await
				.unwrap(),
			0
		);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn link_policies_decide_who_adds_and_who_sees() {
	on_both(async |db| {
		let ada = writer(&db, "Ada").await;
		let grace = writer(&db, "Grace").await;
		let as_ada = as_writer(&db, &ada);
		let as_grace = as_writer(&db, &grace);
		let engines = note(&db, &ada, "Engines", true).await;
		let draft = note(&db, &ada, "Draft", false).await;
		let maths = label(&db, "maths").await;
		let secret = label(&db, "secret").await;

		// Only the writer labels a note.
		let refused = engines
			.labels()
			.add(&as_grace, maths.id, public(0))
			.await
			.unwrap_err();
		assert!(refused.is_forbidden(), "{refused:?}");
		// No one links from a row they can't see: Grace can't see the draft.
		let unseen = draft
			.labels()
			.add(&as_grace, maths.id, public(0))
			.await
			.unwrap_err();
		assert!(unseen.is_not_found(), "{unseen:?}");
		assert_eq!(NoteLabel::query().count(&as_ada).await.unwrap(), 0);

		// A private link is the writer's alone.
		engines
			.labels()
			.add(&as_ada, maths.id, public(0))
			.await
			.unwrap();
		engines
			.labels()
			.add(
				&as_ada,
				secret.id,
				NoteLabelProperties {
					position: 1,
					private: true,
				},
			)
			.await
			.unwrap();
		assert_eq!(engines.labels().count(&as_ada).await.unwrap(), 2);
		assert_eq!(engines.labels().ids(&as_grace).await.unwrap(), [maths.id]);
		assert_eq!(secret.notes().count(&as_grace).await.unwrap(), 0);

		// Grace can't remove Ada's links, even ones she sees.
		assert!(!engines.labels().remove(&as_grace, maths.id).await.unwrap());
		assert_eq!(engines.labels().count(&as_grace).await.unwrap(), 1);

		// Bypassing policies sees everything.
		let system = db.system("checking every link");
		assert_eq!(draft.labels().count(&system).await.unwrap(), 0);
		assert_eq!(engines.labels().count(&system).await.unwrap(), 2);
	})
	.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_table_links_to_itself_and_exclusive_links_stay_single() {
	on_both(async |db| {
		let ada = writer(&db, "Ada").await;
		let grace = writer(&db, "Grace").await;
		let alan = writer(&db, "Alan").await;

		let follow = ada.following().add(&db, grace.id, ()).await.unwrap();
		assert_eq!((follow.follower_id, follow.followed_id), (ada.id, grace.id));
		alan.following().add(&db, grace.id, ()).await.unwrap();
		grace.following().add(&db, ada.id, ()).await.unwrap();

		assert_eq!(ada.following().ids(&db).await.unwrap(), [grace.id]);
		assert_eq!(grace.followers().count(&db).await.unwrap(), 2);
		let followers = grace.followers().all(&db).await.unwrap();
		let mut names: Vec<&str> = followers
			.iter()
			.map(|(writer, _)| writer.name.as_str())
			.collect();
		names.sort_unstable();
		assert_eq!(names, ["Ada", "Alan"]);

		// A label is pinned to one note at most.
		let first = note(&db, &ada, "First", true).await;
		let second = note(&db, &ada, "Second", true).await;
		let maths = label(&db, "maths").await;
		first.pins().add(&db, maths.id, ()).await.unwrap();
		// Pinning it there again is the same link.
		first.pins().add(&db, maths.id, ()).await.unwrap();
		let taken = second.pins().add(&db, maths.id, ()).await.unwrap_err();
		assert!(taken.is_conflict(), "{taken:?}");
		assert_eq!(maths.pinned_on().ids(&db).await.unwrap(), [first.id]);
	})
	.await;
}

/// A world to check, by position.
#[derive(Clone, Debug)]
struct Spec {
	writers: usize,
	/// `(writer, published, deleted)`.
	notes: Vec<(usize, bool, bool)>,
	/// Whether each label is soft-deleted.
	labels: Vec<bool>,
	/// `(note, label, private)`, at most once per pair.
	links: Vec<(usize, usize, bool)>,
	viewer: Option<usize>,
}

fn spec() -> impl Strategy<Value = Spec> {
	(1..=3_usize, 1..=4_usize, 1..=3_usize)
		.prop_flat_map(|(writers, notes, labels)| {
			(
				Just(writers),
				proptest::collection::vec(
					(0..writers, any::<bool>(), proptest::bool::weighted(0.2)),
					notes,
				),
				proptest::collection::vec(proptest::bool::weighted(0.2), labels),
				proptest::collection::vec((0..notes, 0..labels, any::<bool>()), 0..10),
				proptest::option::of(0..writers),
			)
		})
		.prop_map(|(writers, notes, labels, mut links, viewer)| {
			let mut seen = Vec::new();
			links.retain(|&(note, label, _)| {
				let fresh = !seen.contains(&(note, label));
				seen.push((note, label));
				fresh
			});
			Spec {
				writers,
				notes,
				labels,
				links,
				viewer,
			}
		})
}

/// The rules, in plain Rust.
impl Spec {
	fn note_visible(&self, note: usize) -> bool {
		let (writer, published, deleted) = self.notes[note];
		!deleted && (published || self.viewer == Some(writer))
	}

	fn link_visible(&self, &(note, label, private): &(usize, usize, bool)) -> bool {
		let writes = self.viewer == Some(self.notes[note].0);
		self.note_visible(note) && !self.labels[label] && (!private || writes)
	}

	fn labels_of(&self, note: usize) -> Vec<usize> {
		self.links
			.iter()
			.filter(|link| link.0 == note && self.link_visible(link))
			.map(|link| link.1)
			.collect()
	}

	fn notes_of(&self, label: usize) -> Vec<usize> {
		self.links
			.iter()
			.filter(|link| link.1 == label && self.link_visible(link))
			.map(|link| link.0)
			.collect()
	}
}

struct Seeded {
	writers: Vec<Id<Writer>>,
	notes: Vec<Id<Note>>,
	labels: Vec<Id<Label>>,
}

async fn seed(db: &Db, spec: &Spec) -> Seeded {
	let system = db.system("seeding the property test");
	NoteLabel::query().destroy(&system).await.unwrap();
	Pin::query().destroy(&system).await.unwrap();
	Follow::query().destroy(&system).await.unwrap();
	Note::query().destroy(&system).await.unwrap();
	Label::query().destroy(&system).await.unwrap();
	Writer::query().destroy(&system).await.unwrap();

	let mut seeded = Seeded {
		writers: Vec::new(),
		notes: Vec::new(),
		labels: Vec::new(),
	};

	for index in 0..spec.writers {
		let writer = NewWriter {
			name: format!("writer {index}"),
		};
		seeded
			.writers
			.push(writer.insert(&system).await.unwrap().id);
	}

	for (index, &(writer, published, _)) in spec.notes.iter().enumerate() {
		let note = NewNote {
			writer_id: seeded.writers[writer],
			title: format!("note {index}"),
			published,
		};
		seeded.notes.push(note.insert(&system).await.unwrap().id);
	}

	for index in 0..spec.labels.len() {
		let label = NewLabel {
			name: format!("label {index}"),
		};
		seeded.labels.push(label.insert(&system).await.unwrap().id);
	}

	// Links go in while every end is visible; deletions come after.
	for (position, &(note, label, private)) in spec.links.iter().enumerate() {
		leptoskit_db::Links::<NoteLabel, _, _>::forward(seeded.notes[note])
			.add(
				&system,
				seeded.labels[label],
				NoteLabelProperties {
					position: i16::try_from(position).unwrap(),
					private,
				},
			)
			.await
			.unwrap();
	}

	for (index, &(_, _, deleted)) in spec.notes.iter().enumerate() {
		if deleted {
			let mut note = Note::get(&system, seeded.notes[index]).await.unwrap();
			note.delete(&system).await.unwrap();
		}
	}

	for (index, &deleted) in spec.labels.iter().enumerate() {
		if deleted {
			let mut label = Label::get(&system, seeded.labels[index]).await.unwrap();
			label.delete(&system).await.unwrap();
		}
	}

	seeded
}

fn sorted<T: Ord>(mut items: Vec<T>) -> Vec<T> {
	items.sort();
	items
}

async fn agrees_with_the_model(db: &Db, spec: Spec) -> Result<(), TestCaseError> {
	let seeded = seed(db, &spec).await;
	let request = match spec.viewer {
		Some(writer) => db.with_context(Context::new().with(Viewer(seeded.writers[writer]))),
		None => db.clone(),
	};

	for (note, &id) in seeded.notes.iter().enumerate() {
		let links = leptoskit_db::Links::<NoteLabel, _, _>::forward(id);
		let expected: Vec<Id<Label>> = spec
			.labels_of(note)
			.into_iter()
			.map(|label| seeded.labels[label])
			.collect();
		prop_assert_eq!(
			sorted(links.ids(&request).await.unwrap()),
			sorted(expected.clone()),
			"labels of note {}",
			note
		);
		let pairs = leptoskit_db::Links::<NoteLabel, _, _>::forward(id)
			.all(&request)
			.await
			.unwrap();
		prop_assert_eq!(
			sorted(pairs.iter().map(|(label, _)| label.id).collect()),
			sorted(expected),
			"labelled rows of note {}",
			note
		);
	}

	for (label, &id) in seeded.labels.iter().enumerate() {
		let links = leptoskit_db::Links::<NoteLabel, _, _>::backward(id);
		prop_assert_eq!(
			links.count(&request).await.unwrap(),
			spec.notes_of(label).len() as u64,
			"notes of label {}",
			label
		);
	}

	Ok(())
}

fn run_property(db: &Db, runtime: &tokio::runtime::Runtime, cases: u32) {
	let mut runner = TestRunner::new(Config {
		cases,
		failure_persistence: None,
		..Config::default()
	});
	let outcome = runner.run(&spec(), |spec| {
		runtime.block_on(agrees_with_the_model(db, spec))
	});

	if let Err(error) = outcome {
		panic!("{error}");
	}
}

#[test]
fn links_agree_with_the_model_on_sqlite() {
	let runtime = tokio::runtime::Runtime::new().unwrap();
	let db = runtime.block_on(async {
		let db = Db::connect("sqlite::memory:").await.unwrap();
		create_schema(&db).await;
		db
	});
	run_property(&db, &runtime, 96);
}

#[test]
fn links_agree_with_the_model_on_postgres() {
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
