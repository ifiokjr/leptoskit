//! What runs inside the database rather than storing rows: sequences,
//! functions, and the triggers that call them.
//!
//! Apps don't declare these themselves. Schema packs do, such as `live`,
//! which captures every write to a live table: its migration creates the
//! sequences and functions once, and each live table gets triggers that name
//! its columns.

use std::fmt;

use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;

use crate::TableName;

/// A Postgres sequence: numbers handed out in order, never twice, even to
/// transactions that roll back. SQLite has none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sequence {
	/// The sequence's name, with an optional namespace.
	pub name: TableName,
}

impl Sequence {
	/// A sequence starting at 1.
	pub fn new(name: TableName) -> Self {
		Self { name }
	}
}

/// A Postgres function a schema pack owns, written in the language it names.
/// SQLite has none: its triggers hold their statements.
///
/// The migration writes it with `CREATE OR REPLACE FUNCTION`, so a changed
/// body replaces the old one in place. `schema.json` keeps the body and its
/// checksum, so a review shows that it changed, and a snapshot edited by
/// hand fails to load.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Function {
	/// The function's name, with an optional namespace. It takes no
	/// arguments: a trigger function reads its trigger's arguments.
	pub name: TableName,
	/// What it returns, such as `trigger`.
	pub returns: String,
	/// The language its body is written in, such as `plpgsql`.
	pub language: String,
	/// Whether it runs with the privileges of the role that created it, so
	/// that row-level security on the app's tables never stops it.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub security_definer: bool,
	/// Its body, between the dollar quotes.
	pub body: String,
	/// The SHA-256 of `body`, in hex.
	pub checksum: String,
}

/// The dollar quote around a function's body, which the body can't contain.
pub const BODY_QUOTE: &str = "$leptoskit$";

impl Function {
	/// A function with this body, checksummed.
	pub fn new(
		name: TableName,
		returns: impl Into<String>,
		language: impl Into<String>,
		body: impl Into<String>,
	) -> Self {
		let body = body.into();

		Self {
			name,
			returns: returns.into(),
			language: language.into(),
			security_definer: false,
			checksum: Self::checksum_of(&body),
			body,
		}
	}

	/// The same function, run with its owner's privileges.
	#[must_use]
	pub fn security_definer(mut self) -> Self {
		self.security_definer = true;
		self
	}

	/// The checksum a body has.
	pub fn checksum_of(body: &str) -> String {
		format!("{:x}", Sha256::digest(body.as_bytes()))
	}
}

/// When a trigger runs, relative to the write that fires it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerTiming {
	/// Before the row is written.
	Before,
	/// After the row is written.
	After,
}

impl TriggerTiming {
	/// The timing's SQL keyword.
	pub fn sql(self) -> &'static str {
		match self {
			Self::Before => "BEFORE",
			Self::After => "AFTER",
		}
	}
}

/// A write that fires a trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerEvent {
	/// A row is inserted.
	Insert,
	/// A row is changed.
	Update,
	/// A row is deleted.
	Delete,
}

impl TriggerEvent {
	/// The event's SQL keyword.
	pub fn sql(self) -> &'static str {
		match self {
			Self::Insert => "INSERT",
			Self::Update => "UPDATE",
			Self::Delete => "DELETE",
		}
	}
}

/// What a trigger does, which depends on the dialect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum TriggerAction {
	/// Postgres: calls a function, with text arguments it reads from
	/// `TG_ARGV`.
	Function {
		/// The function, which must be in the schema.
		function: TableName,
		/// The arguments, in order.
		#[serde(default, skip_serializing_if = "Vec::is_empty")]
		arguments: Vec<String>,
	},
	/// SQLite: runs statements, which read the row as `OLD` and `NEW`.
	Statements {
		/// Fires only for rows this is true of.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		when: Option<String>,
		/// The statements between `BEGIN` and `END`, each ending in `;`.
		sql: String,
	},
}

/// A trigger: work the database does for every row a write touches, in the
/// writing transaction, whoever writes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trigger {
	/// The trigger's name: unique on its table on Postgres, and in the
	/// whole database on SQLite.
	pub name: String,
	/// The table whose writes fire it.
	pub table: TableName,
	/// When it runs.
	pub timing: TriggerTiming,
	/// The writes that fire it. SQLite allows only one per trigger.
	pub events: Vec<TriggerEvent>,
	/// Postgres only: a constraint trigger, deferred until the transaction
	/// commits (`DEFERRABLE INITIALLY DEFERRED`), as foreign keys can be.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub deferred: bool,
	/// The columns it copies, when it copies rows, as capture triggers do:
	/// what `lk db check` compares and warns about.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub columns: Vec<String>,
	/// What it does.
	pub action: TriggerAction,
}

impl Trigger {
	/// Whether the trigger is about the same table and name as `other`, so
	/// that one replaces the other.
	pub fn same_name(&self, other: &Self) -> bool {
		self.table == other.table && self.name == other.name
	}

	/// What kind of trigger it is, for messages.
	pub fn kind(&self) -> &'static str {
		if self.deferred {
			"constraint trigger"
		} else {
			"trigger"
		}
	}
}

impl fmt::Display for Trigger {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{} {} on {}", self.kind(), self.name, self.table)
	}
}
