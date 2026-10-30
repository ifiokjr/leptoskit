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
	updated_by: Option<&'static str>,
	version: Option<&'static str>,
	/// How to record the source's rows, when it records changes.
	audit: Option<crate::audit::Subject>,
}

impl SoftCascade {
	/// The link `column` from `S` to `T`.
	///
	/// # Panics
	///
	/// At compile time, when `S` can't be soft-deleted. `#[db::table]`
	/// reports that first, with a better message.
	pub const fn of<S: Table, T: crate::Referenced>(column: &'static str) -> Self {
		let Some(deleted_at) = S::DELETED_AT else {
			panic!("`on_soft_delete` needs the table to have a `#[db(deleted_at)]` column");
		};

		Self {
			target: T::referenced_table,
			source: S::table_name,
			column,
			key: S::KEY,
			deleted_at,
			updated_at: S::UPDATED_AT,
			updated_by: S::UPDATED_BY,
			version: S::VERSION,
			audit: crate::audit::Subject::recorded::<S>(),
		}
	}
}

inventory::collect!(SoftCascade);

/// Whether soft-deleting a row of `T` reaches other tables, through links
/// to it or, for a variant of a polymorphic table, to its supertype.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) fn has_cascades<T: Table>() -> bool {
	let tables = cascading_tables::<T>();
	inventory::iter::<SoftCascade>().any(|cascade| tables.contains(&(cascade.target)()))
}

/// The tables whose links follow a soft delete of a `T` row: `T`, and its
/// supertype, which shares its key.
fn cascading_tables<T: Table>() -> Vec<TableName> {
	let mut tables = vec![T::table_name()];
	tables.extend(T::SUPERTYPE.map(|supertype| (supertype.table)()));
	tables
}

/// Soft-deletes what the `T` rows with these keys, just soft-deleted, reach
/// through cascading links.
pub(crate) async fn soft_delete_from<T: Table>(
	connection: &impl Connection,
	keys: Vec<Value>,
	now: Timestamp,
) -> Result<()> {
	for table in cascading_tables::<T>() {
		soft_delete_linked(connection, table, keys.clone(), now).await?;
	}

	Ok(())
}

/// Soft-deletes, as of `now`, the rows that cascading links connect to the
/// rows of `table` with these keys, then the rows connected to those. Rows
/// already deleted are left alone, which also ends cycles. Who is acting
/// goes into `updated_by`, and an audited table records each row.
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

			if let Some(updated_by) = cascade.updated_by {
				let actor = connection
					.access()
					.actor()
					.map_or(Value::Null, |actor| Value::Uuid(actor.uuid()));
				sql.push(", ")
					.identifier(updated_by)
					.push(" = ")
					.value(actor);
			}

			if let Some(version) = cascade.version {
				sql.push(", ")
					.identifier(version)
					.push(" = ")
					.identifier(version)
					.push(" + 1");
			}

			write_linked(&mut sql, cascade, &keys);

			if let Some(subject) = cascade.audit {
				let linked = audited(connection, cascade, subject, &keys, sql).await?;
				pending.push((source, linked));
				continue;
			}

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

/// ` WHERE column IN (keys) AND deleted_at IS NULL`: the live rows linked to
/// these keys.
fn write_linked(sql: &mut Sql, cascade: &SoftCascade, keys: &[Value]) {
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
}

/// Runs a cascade's `update` on an audited table: reads and locks the rows
/// it will change, changes them, and records each one. Returns their keys,
/// for the cascades that follow from them.
async fn audited(
	connection: &impl Connection,
	cascade: &SoftCascade,
	subject: crate::audit::Subject,
	keys: &[Value],
	mut update: Sql,
) -> Result<Vec<Value>> {
	let source = (cascade.source)();
	let mut select = Sql::new(connection.dialect());
	select
		.push("SELECT ")
		.columns(subject.columns)
		.push(" FROM ")
		.table_name(&source);
	write_linked(&mut select, cascade, keys);

	if connection.dialect() == crate::Dialect::Postgres {
		select.push(" FOR UPDATE");
	}

	let key_position = |row: &[Value]| -> Vec<Value> {
		cascade
			.key
			.iter()
			.filter_map(|key| {
				subject
					.columns
					.iter()
					.position(|column| column == key)
					.map(|position| row[position].clone())
			})
			.collect()
	};
	// Keyed by their debug text: `Value` floats aren't `Hash`.
	let mut before: std::collections::HashMap<String, Vec<Value>> = connection
		.fetch(select.finish())
		.await?
		.into_iter()
		.map(|row| (format!("{:?}", key_position(&row)), row))
		.collect();
	update.push(" RETURNING ").columns(subject.columns);
	let mut linked = Vec::new();
	let mut changes = Vec::new();

	for after in connection.fetch(update.finish()).await? {
		let key = key_position(&after);
		let old = before.remove(&format!("{key:?}"));

		if let [single] = key.as_slice() {
			linked.push(single.clone());
		}

		changes.push(crate::audit::Change {
			op: crate::audit::AuditOp::Delete,
			before: old,
			after: Some(after),
		});
	}

	crate::audit::record_changes(connection, subject, changes).await?;
	Ok(linked)
}
