//! Inserting, saving, and deleting single rows. `#[db::table]` generates
//! methods that call these.

use leptoskit_db_schema::Predicate;

use crate::Connection;
use crate::DbError;
use crate::Result;
use crate::Table;
use crate::Value;
use crate::sql::Sql;

/// Inserts a row and returns it as stored.
pub async fn insert<T: Table>(connection: &impl Connection, row: T) -> Result<T> {
	row.validate()?;
	let mut sql = Sql::new(connection.dialect());
	write_insert::<T>(&mut sql, row.to_values()?);
	returning::<T>(connection, sql).await
}

/// Inserts a row, or updates the existing row whose `target` columns match.
/// The key and `created_at` of an existing row are kept.
pub async fn upsert<T: Table>(
	connection: &impl Connection,
	row: T,
	target: &[&'static str],
) -> Result<T> {
	row.validate()?;
	let mut sql = Sql::new(connection.dialect());
	write_insert::<T>(&mut sql, row.to_values()?);
	sql.push(" ON CONFLICT (").columns(target).push(")");

	// A partial unique index only arbitrates the conflict when the statement
	// repeats its predicate.
	let definition = T::definition();
	let predicate = definition
		.indexes
		.iter()
		.find(|index| {
			index.unique
				&& index
					.columns
					.iter()
					.map(String::as_str)
					.eq(target.iter().copied())
		})
		.and_then(|index| index.predicate.clone());

	match predicate {
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
/// `updated_at`.
pub async fn save<T: Table>(connection: &impl Connection, row: &mut T) -> Result<()> {
	row.validate()?;
	row.touch();
	let values = row.to_values()?;
	let mut sql = Sql::new(connection.dialect());
	sql.push("UPDATE ").table::<T>().push(" SET ");
	let mut first = true;

	for (column, value) in T::COLUMNS.iter().zip(&values) {
		if T::KEY.contains(column) || Some(*column) == T::CREATED_AT {
			continue;
		}

		if !first {
			sql.push(", ");
		}

		first = false;
		sql.identifier(column).push(" = ").value(value.clone());
	}

	write_key::<T>(&mut sql, &values);
	expect_one::<T>(connection.execute(sql.finish()).await?)
}

/// Deletes a row: soft-deletes it when the table has a `#[db(deleted_at)]`
/// column, setting that column on `row` too.
pub async fn delete<T: Table>(connection: &impl Connection, row: &mut T) -> Result<()> {
	let Some(column) = T::DELETED_AT else {
		return destroy(connection, row).await;
	};

	row.mark_deleted();
	row.touch();
	let values = row.to_values()?;
	let mut sql = Sql::new(connection.dialect());
	sql.push("UPDATE ").table::<T>().push(" SET ");
	let mut changed = vec![column];
	changed.extend(T::UPDATED_AT);

	for (index, column) in changed.iter().enumerate() {
		if index > 0 {
			sql.push(", ");
		}

		let position = T::COLUMNS
			.iter()
			.position(|candidate| candidate == column)
			.expect("a marker is a column");
		sql.identifier(column)
			.push(" = ")
			.value(values[position].clone());
	}

	write_key::<T>(&mut sql, &values);
	expect_one::<T>(connection.execute(sql.finish()).await?)
}

/// Removes a row for good.
pub async fn destroy<T: Table>(connection: &impl Connection, row: &T) -> Result<()> {
	let values = row.to_values()?;
	let mut sql = Sql::new(connection.dialect());
	sql.push("DELETE FROM ").table::<T>();
	write_key::<T>(&mut sql, &values);
	expect_one::<T>(connection.execute(sql.finish()).await?)
}

fn write_insert<T: Table>(sql: &mut Sql, values: Vec<Value>) {
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

		let position = T::COLUMNS
			.iter()
			.position(|column| column == key)
			.expect("a key is a column");
		sql.identifier(key)
			.push(" = ")
			.value(values[position].clone());
	}
}

fn expect_one<T: Table>(changed: u64) -> Result<()> {
	if changed == 0 {
		Err(DbError::NotFound {
			table: T::flat_name(),
		})
	} else {
		Ok(())
	}
}
