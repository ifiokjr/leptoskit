//! Moving an account's rows to another account, for merges that bring the
//! absorbed account's data along. [`leptoskit_db_schema::account_data`]
//! finds the rows; these statements move each reference by its table's
//! [`MergeStrategy`].
//!
//! They work on tables known only by name, so they skip access policies,
//! and refuse to run on a connection that doesn't: only a system connection
//! (`Db::system`) may move another account's rows.

pub use leptoskit_db_schema::AccountData;
pub use leptoskit_db_schema::AccountReference;
pub use leptoskit_db_schema::KeyedTable;
pub use leptoskit_db_schema::MergeProblem;
pub use leptoskit_db_schema::MergeStrategy;
pub use leptoskit_db_schema::TableName;
pub use leptoskit_db_schema::account_data;
use uuid::Uuid;

use crate::Connection;
use crate::DbError;
use crate::Result;
use crate::Value;
use crate::policy::Access;
use crate::sql::Sql;
use crate::sql::count_of;

/// How many of `reference`'s rows point at `account`: what a merge would
/// move or drop.
///
/// # Errors
///
/// [`DbError::Forbidden`] on a connection that obeys policies, or when the
/// database fails.
pub async fn count_references(
	connection: &impl Connection,
	reference: &AccountReference,
	account: Uuid,
) -> Result<u64> {
	system_only(connection, &reference.table)?;
	let mut sql = Sql::new(connection.dialect());
	sql.push("SELECT count(*) FROM ")
		.table_name(&reference.table)
		.push(" WHERE ")
		.identifier(&reference.column)
		.push(" = ")
		.value(Value::Uuid(account));
	count_of(&connection.fetch(sql.finish()).await?)
}

/// Moves `reference`'s rows from `absorbed` to `survivor` by its strategy,
/// and returns how many rows changed or went. `custom` does nothing here:
/// the app moves those rows itself.
///
/// # Errors
///
/// [`DbError::Forbidden`] on a connection that obeys policies, or the
/// database's error, such as [`DbError::Conflict`] when `reassign` would
/// break a unique constraint. Run it in the merge's transaction, so a
/// failure leaves nothing half-moved.
pub async fn move_references(
	connection: &impl Connection,
	reference: &AccountReference,
	absorbed: Uuid,
	survivor: Uuid,
) -> Result<u64> {
	system_only(connection, &reference.table)?;
	let dialect = connection.dialect();
	let table = &reference.table;
	let column = reference.column.as_str();
	let mut changed = 0;

	let delete_where = |condition: &dyn Fn(&mut Sql)| {
		let mut sql = Sql::new(dialect);
		sql.push("DELETE FROM ")
			.table_name(table)
			.push(" WHERE ")
			.identifier(column)
			.push(" = ")
			.value(Value::Uuid(absorbed));
		condition(&mut sql);
		sql.finish()
	};
	let reassign = || {
		let mut sql = Sql::new(dialect);
		sql.push("UPDATE ")
			.table_name(table)
			.push(" SET ")
			.identifier(column)
			.push(" = ")
			.value(Value::Uuid(survivor))
			.push(" WHERE ")
			.identifier(column)
			.push(" = ")
			.value(Value::Uuid(absorbed));
		sql.finish()
	};

	match reference.strategy {
		MergeStrategy::Reassign => {
			changed += connection.execute(reassign()).await?;
		}
		MergeStrategy::ReassignSkipConflicts => {
			// Drop the rows that would collide with one the survivor already
			// has, under any unique constraint the column is part of.
			if !reference.unique_with.is_empty() {
				changed += connection
					.execute(delete_where(&|sql| {
						sql.push(" AND (");

						for (index, others) in reference.unique_with.iter().enumerate() {
							if index > 0 {
								sql.push(" OR ");
							}

							collides(sql, table, column, others, survivor);
						}

						sql.push(")");
					}))
					.await?;
			}

			changed += connection.execute(reassign()).await?;
		}
		MergeStrategy::KeepSurvivor => {
			changed += connection.execute(delete_where(&|_| {})).await?;
		}
		MergeStrategy::KeepNewest => {
			// The survivor's row goes when the absorbed one is newer, then the
			// absorbed one goes when the survivor still has one, and whatever
			// is left moves.
			let mut sql = Sql::new(dialect);
			sql.push("DELETE FROM ")
				.table_name(table)
				.push(" WHERE ")
				.identifier(column)
				.push(" = ")
				.value(Value::Uuid(survivor))
				.push(" AND EXISTS (SELECT 1 FROM ")
				.table_name(table)
				.push(" AS ")
				.identifier("other")
				.push(" WHERE ")
				.identifier("other")
				.push(".")
				.identifier(column)
				.push(" = ")
				.value(Value::Uuid(absorbed))
				.push(" AND ")
				.identifier("other")
				.push(".")
				.identifier("updated_at")
				.push(" > ");
			qualified(&mut sql, table, "updated_at");
			sql.push(")");
			changed += connection.execute(sql.finish()).await?;
			changed += connection
				.execute(delete_where(&|sql| {
					sql.push(" AND (");
					collides(sql, table, column, &[], survivor);
					sql.push(")");
				}))
				.await?;
			changed += connection.execute(reassign()).await?;
		}
		MergeStrategy::Custom => {}
	}

	Ok(changed)
}

/// `EXISTS` a row of the survivor's with the same values in `others`: the
/// row being looked at would collide with it.
fn collides(sql: &mut Sql, table: &TableName, column: &str, others: &[String], survivor: Uuid) {
	sql.push("EXISTS (SELECT 1 FROM ")
		.table_name(table)
		.push(" AS ")
		.identifier("other")
		.push(" WHERE ")
		.identifier("other")
		.push(".")
		.identifier(column)
		.push(" = ")
		.value(Value::Uuid(survivor));

	for other in others {
		sql.push(" AND ")
			.identifier("other")
			.push(".")
			.identifier(other)
			.push(" = ");
		qualified(sql, table, other);
	}

	sql.push(")");
}

/// `table.column`, naming the outer table of an `UPDATE` or `DELETE`.
fn qualified(sql: &mut Sql, table: &TableName, column: &str) {
	sql.table_name(table).push(".").identifier(column);
}

fn system_only(connection: &impl Connection, table: &TableName) -> Result<()> {
	match connection.access() {
		Access::System { .. } => Ok(()),
		Access::Context(_) => {
			Err(DbError::Forbidden {
				table: table.to_string(),
				message: "moving an account's rows needs a system connection".into(),
			})
		}
	}
}
