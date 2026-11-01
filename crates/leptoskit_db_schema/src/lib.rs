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
mod merge;
mod model;
mod render;
mod triggers;
mod validate;

use sha2::Digest;
use sha2::Sha256;

pub use crate::diff::*;
pub use crate::merge::*;
pub use crate::model::*;
pub use crate::render::*;
pub use crate::triggers::*;
pub use crate::validate::MAX_SPARSE_DIMENSIONS;
pub use crate::validate::MAX_SQLITE_VECTOR_DIMENSIONS;
pub use crate::validate::MAX_VECTOR_DIMENSIONS;
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

/// The fingerprint of a vector's sources, as the triggers that keep a
/// vector column's freshness compute it: the SHA-256, in lowercase hex, of
/// each source as `<bytes>:<text>;`, or `-;` when it's `NULL`. Both
/// databases compute the same fingerprint, so a row's freshness survives a
/// move between them.
pub fn source_fingerprint(sources: &[Option<&str>]) -> String {
	let mut hasher = Sha256::new();

	for source in sources {
		match source {
			Some(text) => {
				hasher.update(text.len().to_string().as_bytes());
				hasher.update(b":");
				hasher.update(text.as_bytes());
			}
			None => hasher.update(b"-"),
		}

		hasher.update(b";");
	}

	format!("{:x}", hasher.finalize())
}

/// [`source_fingerprint`] in SQL, over `row`'s columns (`NEW`, `new`, or
/// the statement's own row when `None`). SQLite computes the hash with
/// `lk_sha256`, which the database layer registers on every connection.
pub fn source_fingerprint_sql(dialect: Dialect, row: Option<&str>, sources: &[String]) -> String {
	let column = |name: &str| {
		match row {
			Some(row) => format!("{row}.{}", quote_identifier(name)),
			None => quote_identifier(name),
		}
	};
	let parts: Vec<String> = sources
		.iter()
		.map(|source| {
			let source = column(source);
			let length = match dialect {
				Dialect::Postgres => format!("octet_length({source})::text"),
				Dialect::Sqlite => format!("octet_length({source})"),
			};
			format!("coalesce({length} || ':' || {source}, '-') || ';'")
		})
		.collect();
	let text = parts.join(" || ");

	match dialect {
		Dialect::Postgres => format!("encode(sha256(convert_to({text}, 'UTF8')), 'hex')"),
		Dialect::Sqlite => format!("lk_sha256({text})"),
	}
}
