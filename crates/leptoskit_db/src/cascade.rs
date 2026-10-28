//! Soft deletes that follow links: `#[db(on_soft_delete = cascade)]`.
//!
//! The database cascades real deletes with `ON DELETE CASCADE`, but a soft
//! delete is an update it knows nothing about. Each cascading link registers
//! itself here, and soft-deleting a row soft-deletes the rows linked to it,
//! and the rows linked to those, in the same transaction.

use leptoskit_db_schema::TableName;

use crate::Connection;
use crate::Result;
use crate::Table;
use crate::Timestamp;
use crate::Value;
use crate::sql::Sql;

/// A link whose rows are soft-deleted with the row they point at.
/// `#[db::table]` registers one per `#[db(on_soft_delete = cascade)]` column.
#[doc(hidden)]
pub struct SoftCascade {
	/// The table the link points at.
	target: fn() -> TableName,
	/// The table the link is declared on.
	source: fn() -> TableName,
	/// The link's column.
	column: &'static str,
	/// The source's key, which further links point at.
	key: &'static [&'static str],
	deleted_at: &'static str,
	updated_at: Option<&'static str>,
	version: Option<&'static str>,
}

impl SoftCascade {
	/// The link `column` from `S` to `T`.
	///
	/// # Panics
	///
	/// At compile time, when `S` can't be soft-deleted. `#[db::table]`
	/// reports that first, with a better message.
	pub const fn of<S: Table, T: Table>(column: &'static str) -> Self {
		let Some(deleted_at) = S::DELETED_AT else {
			panic!("`on_soft_delete` needs the table to have a `#[db(deleted_at)]` column");
		};

		Self {
			target: T::table_name,
			source: S::table_name,
			column,
			key: S::KEY,
			deleted_at,
			updated_at: S::UPDATED_AT,
			version: S::VERSION,
		}
	}
}

inventory::collect!(SoftCascade);

/// Whether soft-deleting a row of `T` reaches other tables.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) fn has_cascades<T: Table>() -> bool {
	let table = T::table_name();
	inventory::iter::<SoftCascade>().any(|cascade| (cascade.target)() == table)
}

/// Soft-deletes, as of `now`, the rows that cascading links connect to the
/// rows of `table` with these keys, then the rows connected to those. Rows
/// already deleted are left alone, which also ends cycles.
pub(crate) async fn soft_delete_linked(
	connection: &impl Connection,
	table: TableName,
	keys: Vec<Value>,
	now: Timestamp,
) -> Result<()> {
	let mut pending = vec![(table, keys)];

	while let Some((table, keys)) = pending.pop() {
		if keys.is_empty() {
			continue;
		}

		for cascade in
			inventory::iter::<SoftCascade>().filter(|cascade| (cascade.target)() == table)
		{
			let source = (cascade.source)();
			let mut sql = Sql::new(connection.dialect());
			sql.push("UPDATE ")
				.table_name(&source)
				.push(" SET ")
				.identifier(cascade.deleted_at)
				.push(" = ")
				.value(Value::Timestamp(now));

			if let Some(updated_at) = cascade.updated_at {
				sql.push(", ")
					.identifier(updated_at)
					.push(" = ")
					.value(Value::Timestamp(now));
			}

			if let Some(version) = cascade.version {
				sql.push(", ")
					.identifier(version)
					.push(" = ")
					.identifier(version)
					.push(" + 1");
			}

			sql.push(" WHERE ").identifier(cascade.column).push(" IN (");

			for (index, key) in keys.iter().enumerate() {
				if index > 0 {
					sql.push(", ");
				}

				sql.value(key.clone());
			}

			sql.push(") AND ")
				.identifier(cascade.deleted_at)
				.push(" IS NULL");

			// Only a table keyed by a single `Id` can be linked to, so only
			// its rows can cascade further.
			if let [key] = cascade.key {
				sql.push(" RETURNING ").identifier(key);
				let linked = connection
					.fetch(sql.finish())
					.await?
					.into_iter()
					.filter_map(|row| row.into_iter().next())
					.collect();
				pending.push((source, linked));
			} else {
				connection.execute(sql.finish()).await?;
			}
		}
	}

	Ok(())
}
