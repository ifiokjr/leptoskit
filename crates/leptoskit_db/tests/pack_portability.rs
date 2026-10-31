//! Exporting refuses a schema whose tables contradict themselves, and a
//! pack's tables that wouldn't work on every dialect, listing all of it at
//! once. Its own test program, since every table linked into a program joins
//! its schema.

#![cfg(feature = "server")]

use leptoskit_db::Dialect;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::table;
use leptoskit_db_schema::SchemaError;

/// An app table whose name breaks the naming rules.
#[table(name = "BadlyNamed")]
pub struct BadlyNamed {
	pub id: Id<Self>,
}

/// A pack table that only works on one dialect.
#[table(name = "events", pack = "probe")]
pub struct Event {
	pub id: Id<Self>,
	#[db(default(sql = "now()"))]
	pub at: Timestamp,
}

/// A pack table that builds on another pack, which packs may, and on an
/// app's table, which they may not.
#[table(name = "notes", pack = "probe_links")]
pub struct Note {
	pub id: Id<Self>,
	pub event_id: Id<Event>,
	pub owner_id: Id<BadlyNamed>,
}

#[test]
fn exporting_lists_every_problem_including_the_packs() {
	let Err(SchemaError::Invalid(problems)) = leptoskit_db::schema(Dialect::Postgres) else {
		panic!("the schema should be refused");
	};
	let messages: Vec<String> = problems.iter().map(ToString::to_string).collect();

	assert!(
		messages
			.iter()
			.any(|message| message.starts_with("BadlyNamed:")),
		"{messages:#?}"
	);
	assert!(
		messages.iter().any(|message| {
			message
				== "probe.events.at: pack `probe`: a pack's column can't default to raw SQL, which \
				    is written for one dialect"
		}),
		"{messages:#?}"
	);
	let linking: Vec<&String> = messages
		.iter()
		.filter(|message| message.contains("pack `probe_links`"))
		.collect();
	assert_eq!(linking.len(), 1, "{messages:#?}");
	assert!(
		linking[0].starts_with("probe_links.notes: ") && linking[0].contains("BadlyNamed"),
		"only the link to the app's table is refused: {messages:#?}"
	);
}
