//! Postgres publications: the tables whose writes logical replication
//! decodes from the write-ahead log.
//!
//! Apps don't declare these themselves. The `live` schema pack does when it
//! captures changes by logical replication rather than triggers: one
//! publication over every live table, which the capture task reads through
//! a replication slot.

use std::fmt;

use serde::Deserialize;
use serde::Serialize;

use crate::TableName;

/// A Postgres publication over whole tables, each with `REPLICA IDENTITY
/// FULL`, so that an update or delete carries the whole row as it was. SQLite
/// has none.
///
/// Every column is published: Postgres refuses to update a table whose
/// publication leaves out columns of a `FULL` replica identity. Which of
/// them a reader may copy is what each table's `columns` say, and the
/// migration keeps that in the publication's comment, as JSON, so a reader
/// learns it from the database rather than from its own build.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Publication {
	/// The publication's name. Publications belong to the database rather
	/// than a namespace.
	pub name: String,
	/// The published tables, sorted by name.
	pub tables: Vec<PublishedTable>,
}

/// One table of a [`Publication`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedTable {
	/// The table.
	pub table: TableName,
	/// Its key columns, in order, which name a changed row.
	pub key: Vec<String>,
	/// The columns a reader may copy out of a changed row: for the change
	/// log, every column but `secret` and `live = skip` ones.
	pub columns: Vec<String>,
}

impl Publication {
	/// A publication over these tables, sorted by name.
	pub fn new(name: impl Into<String>, mut tables: Vec<PublishedTable>) -> Self {
		tables.sort_by(|left, right| left.table.cmp(&right.table));

		Self {
			name: name.into(),
			tables,
		}
	}

	/// The published table with this name, if there is one.
	pub fn table(&self, name: &TableName) -> Option<&PublishedTable> {
		self.tables.iter().find(|table| &table.table == name)
	}

	/// The comment the migration gives the publication: each table's key and
	/// copied columns, as JSON keyed by the table's name, such as
	/// `{"posts":{"key":["id"],"columns":["id","title"]}}`.
	pub fn comment(&self) -> String {
		let tables: serde_json::Map<String, serde_json::Value> = self
			.tables
			.iter()
			.map(|table| {
				(
					table.table.to_string(),
					serde_json::json!({ "key": table.key, "columns": table.columns }),
				)
			})
			.collect();
		serde_json::Value::Object(tables).to_string()
	}

	/// The tables a comment written by [`Publication::comment`] describes,
	/// sorted by name, or `None` when it isn't one.
	pub fn tables_in_comment(comment: &str) -> Option<Vec<PublishedTable>> {
		let serde_json::Value::Object(tables) = serde_json::from_str(comment).ok()? else {
			return None;
		};
		let names = |value: &serde_json::Value| -> Option<Vec<String>> {
			value
				.as_array()?
				.iter()
				.map(|name| name.as_str().map(ToString::to_string))
				.collect()
		};

		let mut tables: Vec<PublishedTable> = tables
			.iter()
			.map(|(name, table)| {
				Some(PublishedTable {
					table: TableName::parse(name),
					key: names(table.get("key")?)?,
					columns: names(table.get("columns")?)?,
				})
			})
			.collect::<Option<_>>()?;
		tables.sort_by(|left, right| left.table.cmp(&right.table));
		Some(tables)
	}
}

impl fmt::Display for Publication {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let tables: Vec<String> = self
			.tables
			.iter()
			.map(|table| table.table.to_string())
			.collect();
		write!(f, "publication {} ({})", self.name, tables.join(", "))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_comment_reads_back_as_the_tables() {
		let publication = Publication::new(
			"leptoskit_live",
			vec![
				PublishedTable {
					table: TableName::namespaced("billing", "invoices"),
					key: vec!["id".into()],
					columns: vec!["id".into(), "total".into()],
				},
				PublishedTable {
					table: TableName::new("posts"),
					key: vec!["a".into(), "b".into()],
					columns: Vec::new(),
				},
			],
		);
		let tables = Publication::tables_in_comment(&publication.comment()).unwrap();
		assert_eq!(tables, publication.tables);
		assert_eq!(
			publication.to_string(),
			"publication leptoskit_live (posts, billing.invoices)"
		);
		assert_eq!(Publication::tables_in_comment("not json"), None);
		assert_eq!(Publication::tables_in_comment("{\"x\":{}}"), None);
	}
}
