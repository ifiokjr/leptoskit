//! The database. Each table is a Rust struct here: the server reads and writes
//! the full row, and clients only ever get its `Public…` type.
//!
//! After changing a table, run `leptoskit db migration create <name>` and
//! commit what it writes under `migrations/`.

use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::table;

/// Something worth remembering. Clients see it as `PublicNote`.
#[table(name = "notes", public)]
pub struct Note {
	pub id: Id<Note>,
	/// What the note says.
	#[db(min_len = 1, max_len = 500)]
	pub body: String,
	#[db(created_at)]
	pub created_at: Timestamp,
}

leptoskit_db::export_schema!();

/// Every migration, embedded for the server to apply at startup.
#[cfg(feature = "server")]
pub static MIGRATIONS: &[leptoskit_db::Migration] = leptoskit_db::migrations!("../migrations");
