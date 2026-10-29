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
}
