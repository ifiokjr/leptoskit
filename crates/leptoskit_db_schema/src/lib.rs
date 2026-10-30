//! The schema model, snapshot format, diff, and SQL renderers behind
//! leptoskit's generated database migrations.
//!
//! An app declares its tables as Rust structs. `leptoskit db` exports them as
//! a [`Schema`], compares that with the snapshot committed beside the latest
//! migration, and writes the difference as SQL:
//!
//! ```text
//! previous snapshot ──┐
//!                     ├── diff ──► Plan ──► render ──► migration.sql
//! target schema ──────┘                                schema.json (the target, without hints)
//! ```
//!
//! Everything here is pure: no database connection, no files. The baseline is
//! always a committed snapshot, never a live database, so generating a
//! migration gives the same result on every machine.
//!
//! ```
//! use leptoskit_db_schema::*;
//!
//! let users = TableName::new("users");
//! let previous = Schema::empty(Dialect::Postgres);
//! let target = Schema::new(
//! 	Dialect::Postgres,
//! 	vec![Table::new(
//! 		users.clone(),
//! 		vec![
//! 			Column::new("id", ColumnType::Uuid),
//! 			Column::new("handle", ColumnType::Text { max_len: Some(32) }),
//! 		],
//! 		vec!["id".into()],
//! 	)],
//! );
//!
//! let plan = diff(&previous, &target, &mut RequireHints)?;
//! let migration = render(&plan, &previous, &target);
//! assert!(migration.sql().contains(r#"CREATE TABLE "users""#));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

mod diff;
pub mod live;
mod model;
mod render;
mod triggers;
mod validate;

use sha2::Digest;
use sha2::Sha256;

pub use crate::diff::*;
pub use crate::model::*;
pub use crate::render::*;
pub use crate::triggers::*;
pub use crate::validate::Problem;

/// The longest identifier Postgres keeps; longer ones are silently truncated,
/// so generated names stay within it.
pub const MAX_IDENTIFIER_LEN: usize = 63;

/// Why a schema or snapshot can't be used.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
	/// The snapshot isn't valid JSON for this format.
	#[error("the schema snapshot isn't valid: {0}")]
	Json(#[source] serde_json::Error),
	/// The snapshot was written by a newer or older format.
	#[error(
		"the schema snapshot uses format {found}, and this version of leptoskit reads format \
		 {supported}"
	)]
	UnsupportedFormat {
		/// The snapshot's format.
		found: u32,
		/// The format this crate reads.
		supported: u32,
	},
	/// The schema contradicts itself.
	#[error("the schema has {} problem(s):\n{}", .0.len(), validate::list(.0))]
	Invalid(Vec<Problem>),
}

/// The conventional name for a constraint or index: the table's flat name,
/// the columns, and a suffix (`pkey`, `key`, `idx`, `fkey`, `check`), joined
/// by `_`.
///
/// Names longer than [`MAX_IDENTIFIER_LEN`] are shortened and end in a hash
/// of the full name, so they stay unique and stable.
pub fn constraint_name(table: &TableName, columns: &[&str], suffix: &str) -> String {
	let mut name = table.flat();

	for column in columns {
		name.push('_');
		name.push_str(column);
	}

	name.push('_');
	name.push_str(suffix);
	shorten_identifier(&name)
}

/// Shortens an identifier to [`MAX_IDENTIFIER_LEN`] bytes, replacing the end
/// with a hash of the whole so that different long names stay different.
pub fn shorten_identifier(name: &str) -> String {
	if name.len() <= MAX_IDENTIFIER_LEN {
		return name.to_string();
	}

	let hash = Sha256::digest(name.as_bytes());
	let hash = format!("{hash:x}");
	let suffix = &hash[..8];
	// Identifiers are ASCII, so any byte index is a character boundary.
	let keep = MAX_IDENTIFIER_LEN - suffix.len() - 1;
	format!("{}_{suffix}", &name[..keep])
}

/// Quotes an identifier for either dialect.
pub fn quote_identifier(identifier: &str) -> String {
	format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// Quotes a string literal for either dialect.
pub fn quote_literal(value: &str) -> String {
	format!("'{}'", value.replace('\'', "''"))
}
