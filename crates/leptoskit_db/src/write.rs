//! Inserting, saving, and deleting single rows. `#[db::table]` generates
//! methods that call these.
//!
//! On a table with access policies, a write is filtered by what the request
//! may change and then checked: the stored result must be one the request
//! may insert or update, or the write rolls back with
//! [`DbError::Forbidden`]. The check reads the row as stored, defaults and
//! all, in the same transaction.

use leptoskit_db_schema::Predicate;

use crate::Action;
use crate::Connection;
use crate::DbError;
use crate::Expr;
use crate::Result;
use crate::Table;
use crate::Value;
use crate::connection::atomically;
use crate::expr::Comparison;
use crate::policy;
use crate::sql::Sql;

/// Inserts a row and returns it as stored.
pub async fn insert<T: Table>(connection: &impl Connection, row: T) -> Result<T> {
	row.validate()?;

	let Some(check) = policy::condition::<T>(connection.access(), Action::Insert) else {
		return write_insert(connection, row).await;
	};

	atomically!(connection, |tx| checked_insert(tx, row, &check).await)
}

/// Inserts, then checks the stored row. Errors return from here rather than
/// from inside `atomically!`, which must see them to roll back.
async fn checked_insert<T: Table>(connection: &impl Connection, row: T, check: &Expr) -> Result<T> {
	let stored = write_insert(connection, row).await?;
	let key = policy::key_of::<T>(&stored.to_values()?);
	policy::verify::<T>(connection, &[key], Action::Insert, check).await?;
	Ok(stored)
}

async fn write_insert<T: Table>(connection: &impl Connection, row: T) -> Result<T> {
	let mut sql = Sql::new(connection.dialect());
	insert_statement::<T>(&mut sql, row.to_values()?);
	returning::<T>(connection, sql).await
}

/// Inserts a row, or updates the existing row whose `target` columns match.
/// The key and `created_at` of an existing row are kept.
///
/// With policies, overwriting an existing row needs it to be one the request
/// may update, and the result must be one it may insert or update.
pub async fn upsert<T: Table>(
	connection: &impl Connection,
	row: T,
	target: &[&'static str],
) -> Result<T> {
	row.validate()?;
	let insert = policy::condition::<T>(connection.access(), Action::Insert);
	let update = policy::condition::<T>(connection.access(), Action::Update);

	if insert.is_none() && update.is_none() {
		return write_upsert(connection, row, target).await;
	}

	atomically!(connection, |tx| {
		checked_upsert(tx, row, target, insert.as_ref(), update.as_ref()).await
	})
}

async fn checked_upsert<T: Table>(
	connection: &impl Connection,
	row: T,
	target: &[&'static str],
	insert: Option<&Expr>,
	update: Option<&Expr>,
) -> Result<T> {
	let conflict = conflict_condition::<T>(&row.to_values()?, target);
	let overwrites = policy::count::<T>(connection, &conflict).await? > 0;

	if overwrites {
		if let Some(update) = update {
			if policy::count::<T>(connection, &(conflict.clone() & update.clone())).await? == 0 {
				return Err(policy::refuse::<T>(connection, &conflict, Action::Update).await?);
			}
		}
	}

	let stored = write_upsert(connection, row, target).await?;
	let key = policy::key_of::<T>(&stored.to_values()?);
	let (action, check) = if overwrites {
		(Action::Update, update)
	} else {
		(Action::Insert, insert)
	};

	if let Some(check) = check {
		policy::verify::<T>(connection, &[key], action, check).await?;
	}

	Ok(stored)
}

/// The existing rows an upsert would overwrite: the target columns' values,
/// within the unique index's predicate when it's partial.
fn conflict_condition<T: Table>(values: &[Value], target: &[&'static str]) -> Expr {
	let matches = target
		.iter()
		.map(|column| {
			Expr::Compare {
				column,
				comparison: Comparison::Equal,
				value: values[position::<T>(column)].clone(),
			}
		})
		.reduce(|left, right| left & right)
		.unwrap_or_else(Expr::never);

	match unique_predicate::<T>(target) {
		Some(Predicate::IsNull(column)) => {
			matches
				& Expr::Null {
					column: static_column::<T>(&column),
					negated: false,
				}
		}
		Some(Predicate::IsNotNull(column)) => {
			matches
				& Expr::Null {
					column: static_column::<T>(&column),
					negated: true,
				}
		}
		Some(Predicate::Sql(_)) | None => matches,
	}
}

/// The predicate of the partial unique index over `target`, if it is one.
fn unique_predicate<T: Table>(target: &[&'static str]) -> Option<Predicate> {
	T::definition()
		.indexes
		.into_iter()
		.find(|index| {
			index.unique
				&& index
					.columns
					.iter()
					.map(String::as_str)
					.eq(target.iter().copied())
		})
		.and_then(|index| index.predicate)
}

/// The table's own name for a column, which outlives the definition.
fn static_column<T: Table>(column: &str) -> &'static str {
	T::COLUMNS[position::<T>(column)]
}

async fn write_upsert<T: Table>(
	connection: &impl Connection,
	row: T,
	target: &[&'static str],
) -> Result<T> {
	let mut sql = Sql::new(connection.dialect());
	insert_statement::<T>(&mut sql, row.to_values()?);
	sql.push(" ON CONFLICT (").columns(target).push(")");

	// A partial unique index only arbitrates the conflict when the statement
	// repeats its predicate.
	match unique_predicate::<T>(target) {
		Some(Predicate::IsNull(column)) => {
			sql.push(" WHERE ").identifier(&column).push(" IS NULL");
		}
		Some(Predicate::IsNotNull(column)) => {
			sql.push(" WHERE ").identifier(&column).push(" IS NOT NULL");
		}
		Some(Predicate::Sql(predicate)) => {
			sql.push(" WHERE ").push(&predicate);
		}
		None => {}
	}

	let updated: Vec<&str> = T::COLUMNS
		.iter()
		.copied()
		.filter(|column| {
			!T::KEY.contains(column) && !target.contains(column) && Some(*column) != T::CREATED_AT
		})
		.collect();
	// With nothing else to change, a no-op update still returns the row.
	let updated = if updated.is_empty() {
		target.to_vec()
	} else {
		updated
	};
	sql.push(" DO UPDATE SET ");

	for (index, column) in updated.iter().enumerate() {
		if index > 0 {
			sql.push(", ");
		}

		sql.identifier(column)
			.push(" = EXCLUDED.")
			.identifier(column);
	}

	returning::<T>(connection, sql).await
}

/// Writes every column of an existing row, matched by its key, and bumps
/// `updated_at`. A versioned row is only written while its version is the
/// one it was read with; otherwise the save fails with
/// [`DbError::Stale`]. With policies, the row must be one the request may
/// change, and so must what it becomes.
pub async fn save<T: Table>(connection: &impl Connection, row: &mut T) -> Result<()> {
	row.validate()?;
	row.touch();
	let values = row.to_values()?;

	match policy::condition::<T>(connection.access(), Action::Update) {
		None => write_save::<T>(connection, &values, None).await?,
		Some(check) => {
			atomically!(connection, |tx| {
				checked_save::<T>(tx, &values, &check).await
			})?;
		}
	}

	row.bump_version();
	Ok(())
}

/// Saves the row the request may change, then checks what it became.
async fn checked_save<T: Table>(
	connection: &impl Connection,
	values: &[Value],
	check: &Expr,
) -> Result<()> {
	write_save::<T>(connection, values, Some(check)).await?;
	policy::verify::<T>(
		connection,
		&[policy::key_of::<T>(values)],
		Action::Update,
		check,
	)
	.await
}

async fn write_save<T: Table>(
	connection: &impl Connection,
	values: &[Value],
	filter: Option<&Expr>,
) -> Result<()> {
	let mut sql = Sql::new(connection.dialect());
	sql.push("UPDATE ").table::<T>().push(" SET ");
	let mut first = true;

	for (column, value) in T::COLUMNS.iter().zip(values) {
		if T::KEY.contains(column) || Some(*column) == T::CREATED_AT {
			continue;
		}

		if !first {
			sql.push(", ");
		}

		first = false;
		sql.identifier(column).push(" = ");

		if Some(*column) == T::VERSION {
			sql.identifier(column).push(" + 1");
		} else {
			sql.value(value.clone());
		}
	}

	write_key::<T>(&mut sql, values);

	if let Some(version) = T::VERSION {
		sql.push(" AND ")
			.identifier(version)
			.push(" = ")
			.value(values[position::<T>(version)].clone());
	}

	write_filter(&mut sql, filter)?;

	if connection.execute(sql.finish()).await? == 0 {
		return Err(policy::missing::<T>(connection, values, Action::Update, true).await?);
	}

	Ok(())
}

/// Where a column sits in [`Table::COLUMNS`].
fn position<T: Table>(column: &str) -> usize {
	T::COLUMNS
		.iter()
		.position(|candidate| *candidate == column)
		.expect("a marker is a column")
}

/// Deletes a row: soft-deletes it when the table has a `#[db(deleted_at)]`
/// column, setting that column on `row` too. Rows linked to it with
/// `on_soft_delete = cascade` are soft-deleted with it, in one transaction.
/// With policies, the row must be one the request may delete.
pub async fn delete<T: Table>(connection: &impl Connection, row: &mut T) -> Result<()> {
	if T::DELETED_AT.is_none() {
		return destroy(connection, row).await;
	}

	row.mark_deleted();
	row.touch();
	let filter = policy::condition::<T>(connection.access(), Action::Delete);

	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	let deleted = if crate::cascade::has_cascades::<T>() {
		atomically!(connection, |tx| soft_delete(tx, row, filter.as_ref()).await)
	} else {
		soft_delete(connection, row, filter.as_ref()).await
	};
	#[cfg(not(any(feature = "postgres", feature = "sqlite")))]
	let deleted = soft_delete(connection, row, filter.as_ref()).await;

	deleted?;
	row.bump_version();
	Ok(())
}

/// Writes a row's soft delete, already marked on `row`, then follows its
/// cascading links. Cascades are integrity, not access: they reach linked
/// rows whatever those tables' policies say.
async fn soft_delete<T: Table>(
	connection: &impl Connection,
	row: &T,
	filter: Option<&Expr>,
) -> Result<()> {
	let column = T::DELETED_AT.expect("only tables with a soft-delete column soft-delete");
	let values = row.to_values()?;
	let mut sql = Sql::new(connection.dialect());
	sql.push("UPDATE ").table::<T>().push(" SET ");
	let mut changed = vec![column];
	changed.extend(T::UPDATED_AT);

	for (index, column) in changed.iter().enumerate() {
		if index > 0 {
			sql.push(", ");
		}

		sql.identifier(column)
			.push(" = ")
			.value(values[position::<T>(column)].clone());
	}

	if let Some(version) = T::VERSION {
		sql.push(", ")
			.identifier(version)
			.push(" = ")
			.identifier(version)
			.push(" + 1");
	}

	write_key::<T>(&mut sql, &values);
	write_filter(&mut sql, filter)?;

	if connection.execute(sql.finish()).await? == 0 {
		return Err(policy::missing::<T>(connection, &values, Action::Delete, false).await?);
	}

	if let ([key], Value::Timestamp(now)) = (T::KEY, &values[position::<T>(column)]) {
		crate::cascade::soft_delete_linked(
			connection,
			T::table_name(),
			vec![values[position::<T>(key)].clone()],
			*now,
		)
		.await?;
	}

	Ok(())
}

/// Removes a row for good. With policies, the row must be one the request
/// may delete.
pub async fn destroy<T: Table>(connection: &impl Connection, row: &T) -> Result<()> {
	let values = row.to_values()?;
	let filter = policy::condition::<T>(connection.access(), Action::Delete);
	let mut sql = Sql::new(connection.dialect());
	sql.push("DELETE FROM ").table::<T>();
	write_key::<T>(&mut sql, &values);
	write_filter(&mut sql, filter.as_ref())?;

	if connection.execute(sql.finish()).await? == 0 {
		return Err(policy::missing::<T>(connection, &values, Action::Delete, false).await?);
	}

	Ok(())
}

fn insert_statement<T: Table>(sql: &mut Sql, values: Vec<Value>) {
	sql.push("INSERT INTO ")
		.table::<T>()
		.push(" (")
		.columns(T::COLUMNS)
		.push(") VALUES (");

	for (index, value) in values.into_iter().enumerate() {
		if index > 0 {
			sql.push(", ");
		}

		sql.value(value);
	}

	sql.push(")");
}

async fn returning<T: Table>(connection: &impl Connection, mut sql: Sql) -> Result<T> {
	sql.push(" RETURNING ").columns(T::COLUMNS);
	let row = connection.fetch(sql.finish()).await?.into_iter().next();
	let row = row.ok_or_else(|| {
		DbError::NotFound {
			table: T::flat_name(),
		}
	})?;
	Ok(T::from_values(row)?)
}

/// `WHERE key = …` for a row's key columns.
fn write_key<T: Table>(sql: &mut Sql, values: &[Value]) {
	sql.push(" WHERE ");

	for (index, key) in T::KEY.iter().enumerate() {
		if index > 0 {
			sql.push(" AND ");
		}

		sql.identifier(key)
			.push(" = ")
			.value(values[position::<T>(key)].clone());
	}
}

/// `AND (…)` for a policy's filter, after `write_key`.
fn write_filter(sql: &mut Sql, filter: Option<&Expr>) -> Result<()> {
	if let Some(filter) = filter {
		sql.push(" AND (");
		filter.write(sql)?;
		sql.push(")");
	}

	Ok(())
}
