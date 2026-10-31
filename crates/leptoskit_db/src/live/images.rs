//! Checking live changes against a subscriber's policy: the rows a change
//! saw, which aren't stored any more, read as if they were.

use leptoskit_db_schema::Dialect;

use crate::Action;
use crate::Result;
use crate::Table;
use crate::Value;
use crate::policy::Access;
use crate::sql::Sql;
use crate::sql::Statement;

/// The column that numbers each image in [`visible_images`]'s statements.
const IMAGE: &str = "lk_live_image";

/// The alias the images take, as rows of `T`.
const IMAGES: &str = "lk_live";

/// The most values one statement binds, well inside both databases' limits.
const MAX_VALUES: usize = 20_000;

/// How many of the rows `images` stands for meet `condition`: `SELECT
/// count(*) FROM (SELECT … AS col, …) AS lk_live WHERE condition`. Checks a
/// row that isn't stored, such as one about to be inserted.
#[cfg(feature = "postgres")]
pub(crate) fn image_count<T: Table>(
	dialect: Dialect,
	image: &[Value],
	condition: &crate::Expr,
) -> Result<Statement> {
	let types = column_types::<T>();
	let mut sql = Sql::new(dialect);
	sql.root_alias(&T::table_name(), IMAGES);
	sql.push("SELECT count(*) FROM (SELECT ");
	write_image::<T>(&mut sql, image, &types);
	sql.push(") AS ").identifier(IMAGES).push(" WHERE (");
	condition.write(&mut sql)?;
	sql.push(")");
	Ok(sql.finish())
}

/// Each column's Postgres type, which an image's values are cast to.
fn column_types<T: Table>() -> Vec<Option<String>> {
	let definition = T::definition();
	T::COLUMNS
		.iter()
		.map(|name| {
			definition
				.column(name)
				.map(|column| leptoskit_db_schema::postgres_type(&column.ty))
		})
		.collect()
}

/// `value AS column, …` for one image, cast on Postgres.
fn write_image<T: Table>(sql: &mut Sql, image: &[Value], types: &[Option<String>]) {
	let dialect = sql.dialect();

	for (index, ((column, value), ty)) in T::COLUMNS.iter().zip(image).zip(types).enumerate() {
		if index > 0 {
			sql.push(", ");
		}

		match (dialect, ty) {
			(Dialect::Postgres, Some(ty)) => {
				sql.push("CAST(")
					.value(value.clone())
					.push(" AS ")
					.push(ty)
					.push(")");
			}
			_ => {
				sql.value(value.clone());
			}
		}

		sql.push(" AS ").identifier(column);
	}
}

/// Statements that return the number of each image, a row of `T`, that
/// `access` may read: `SELECT image FROM (SELECT 0 AS image, … UNION ALL
/// …) AS lk_live WHERE policy`. `None` when nothing restricts reading `T`,
/// so every image is readable. Images are checked against the policy
/// alone: a soft-deleted row is one its readers could ask for.
pub(crate) fn visible_images<T: Table>(
	access: &Access,
	dialect: Dialect,
	images: &[&[Value]],
) -> Result<Option<Vec<Statement>>> {
	let Some(policy) = crate::policy::condition::<T>(access, Action::Select) else {
		return Ok(None);
	};
	let types = column_types::<T>();
	let per_statement = (MAX_VALUES / T::COLUMNS.len().max(1)).max(1);
	let mut statements = Vec::new();

	for (chunk, batch) in images.chunks(per_statement).enumerate() {
		let mut sql = Sql::new(dialect);
		sql.root_alias(&T::table_name(), IMAGES);
		sql.push("SELECT ").identifier(IMAGE).push(" FROM (");

		for (offset, image) in batch.iter().enumerate() {
			if offset > 0 {
				sql.push(" UNION ALL ");
			}

			let number = chunk * per_statement + offset;
			sql.push(&format!("SELECT {number} AS "))
				.identifier(IMAGE)
				.push(", ");
			write_image::<T>(&mut sql, image, &types);
		}

		sql.push(") AS ").identifier(IMAGES).push(" WHERE (");
		policy.write(&mut sql)?;
		sql.push(")");
		statements.push(sql.finish());
	}

	Ok(Some(statements))
}

/// A logged image's values in `columns` order, read by `definition`'s column
/// types. A column the image doesn't hold, such as a `secret` one, is
/// `NULL`, so a policy that reads it fails closed.
pub(crate) fn logged_values(
	definition: &leptoskit_db_schema::Table,
	columns: &[&str],
	image: &serde_json::Map<String, serde_json::Value>,
	dialect: Dialect,
) -> Result<Vec<Value>> {
	columns
		.iter()
		.map(|name| {
			match (definition.column(name), image.get(*name)) {
				(Some(column), Some(json)) => logged_value(&column.ty, json, dialect),
				_ => Ok(Value::Null),
			}
		})
		.collect()
}

/// A logged key's values, in the table's key column order.
pub(crate) fn logged_key(
	definition: &leptoskit_db_schema::Table,
	key: &[serde_json::Value],
	dialect: Dialect,
) -> Result<Vec<Value>> {
	definition
		.primary_key
		.iter()
		.zip(key)
		.map(|(name, json)| {
			match definition.column(name) {
				Some(column) => logged_value(&column.ty, json, dialect),
				None => Ok(Value::Null),
			}
		})
		.collect()
}

/// One column's value as a capture trigger wrote it into JSON: Postgres
/// with `to_jsonb`, SQLite with its blobs as hex. Values that can't be
/// bound back the way the column is read, such as a Postgres shape, are
/// `NULL`.
fn logged_value(
	ty: &leptoskit_db_schema::ColumnType,
	json: &serde_json::Value,
	dialect: Dialect,
) -> Result<Value> {
	use leptoskit_db_schema::ColumnType;
	use serde_json::Value as Json;

	let mismatch = || {
		crate::DbError::Decode(crate::DecodeError::custom(format!(
			"the change log holds {json} for a {ty} column"
		)))
	};

	Ok(match (ty, json) {
		(_, Json::Null) => Value::Null,
		(ColumnType::Bool, Json::Bool(value)) => Value::Bool(*value),
		(ColumnType::Bool, Json::Number(number)) => Value::Bool(number.as_i64() != Some(0)),
		(ColumnType::Int16 | ColumnType::Int32 | ColumnType::Int64, Json::Number(number)) => {
			Value::Int(number.as_i64().ok_or_else(mismatch)?)
		}
		(ColumnType::Float32 | ColumnType::Float64, Json::Number(number)) => {
			Value::Float(number.as_f64().ok_or_else(mismatch)?)
		}
		// Postgres writes the floats JSON has no numbers for as text.
		(ColumnType::Float32 | ColumnType::Float64, Json::String(text)) => {
			Value::Float(text.parse().map_err(|_| mismatch())?)
		}
		(ColumnType::Decimal { .. }, Json::Number(number)) if dialect == Dialect::Sqlite => {
			Value::Text(number.to_string())
		}
		(ColumnType::Decimal { .. }, Json::String(text)) if dialect == Dialect::Sqlite => {
			Value::Text(text.clone())
		}
		(ColumnType::Text { .. }, Json::String(text)) => Value::Text(text.clone()),
		(ColumnType::Uuid, Json::String(text)) => {
			Value::Uuid(uuid::Uuid::parse_str(text).map_err(|_| mismatch())?)
		}
		(ColumnType::Timestamp, Json::String(text)) => {
			Value::Timestamp(super::log::parse_timestamp(text).ok_or_else(mismatch)?)
		}
		(ColumnType::Date, Json::String(text)) => {
			Value::Date(
				chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| mismatch())?,
			)
		}
		(ColumnType::Bytes, Json::String(text)) => Value::Bytes(hex(text).ok_or_else(mismatch)?),
		(ColumnType::Geometry { .. } | ColumnType::Geography { .. }, Json::String(text))
			if dialect == Dialect::Sqlite =>
		{
			Value::Bytes(hex(text).ok_or_else(mismatch)?)
		}
		(ColumnType::Json | ColumnType::Array { .. }, json) => Value::Json(json.clone()),
		(
			ColumnType::Decimal { .. } | ColumnType::Geometry { .. } | ColumnType::Geography { .. },
			_,
		) => Value::Null,
		_ => return Err(mismatch()),
	})
}

/// Hex, as SQLite's `hex()` writes it, or as Postgres writes `bytea` after
/// `\x`.
fn hex(text: &str) -> Option<Vec<u8>> {
	let text = text.strip_prefix("\\x").unwrap_or(text);

	if text.len() % 2 != 0 {
		return None;
	}

	(0..text.len())
		.step_by(2)
		.map(|start| u8::from_str_radix(text.get(start..start + 2)?, 16).ok())
		.collect()
}

#[cfg(test)]
mod tests {
	use leptoskit_db_schema::Column;
	use leptoskit_db_schema::ColumnType;
	use leptoskit_db_schema::TableName;
	use serde_json::json;

	use super::*;

	#[test]
	fn logged_images_read_back_as_the_values_rows_hold() {
		let table = leptoskit_db_schema::Table::new(
			TableName::new("things"),
			vec![
				Column::new("id", ColumnType::Uuid),
				Column::new("on", ColumnType::Bool),
				Column::new("at", ColumnType::Timestamp),
				Column::new("day", ColumnType::Date),
				Column::new("blob", ColumnType::Bytes),
				Column::new("ratio", ColumnType::Float64),
				Column::new("secret", ColumnType::TEXT),
			],
			vec!["id".into()],
		);
		let columns = ["id", "on", "at", "day", "blob", "ratio", "secret"];
		let id = uuid::Uuid::from_u128(1);
		let at = chrono::DateTime::parse_from_rfc3339("2026-10-31T12:00:00.5Z")
			.unwrap()
			.to_utc();
		let expected = vec![
			Value::Uuid(id),
			Value::Bool(true),
			Value::Timestamp(at),
			Value::Date(at.date_naive()),
			Value::Bytes(vec![1, 171]),
			Value::Float(f64::NAN),
			Value::Null,
		];
		let same = |values: Vec<Value>| {
			assert_eq!(values.len(), expected.len());

			for (value, expected) in values.iter().zip(&expected) {
				match (value, expected) {
					(Value::Float(value), Value::Float(_)) => assert!(value.is_nan()),
					_ => assert_eq!(value, expected),
				}
			}
		};

		// As Postgres's `to_jsonb` writes the row.
		let postgres = json!({
			"id": id.to_string(), "on": true, "at": "2026-10-31T12:00:00.5+00:00",
			"day": "2026-10-31", "blob": "\\x01ab", "ratio": "NaN",
		});
		same(
			logged_values(
				&table,
				&columns,
				postgres.as_object().unwrap(),
				Dialect::Postgres,
			)
			.unwrap(),
		);

		// As SQLite's capture triggers write it.
		let sqlite = json!({
			"id": id.simple().to_string(), "on": 1, "at": "2026-10-31T12:00:00.500Z",
			"day": "2026-10-31", "blob": "01ab", "ratio": "NaN",
		});
		same(
			logged_values(
				&table,
				&columns,
				sqlite.as_object().unwrap(),
				Dialect::Sqlite,
			)
			.unwrap(),
		);

		assert_eq!(
			logged_key(&table, &[json!(id.simple().to_string())], Dialect::Sqlite).unwrap(),
			[Value::Uuid(id)]
		);
		let wrong = json!({ "on": "yes" });
		assert!(
			logged_values(
				&table,
				&columns,
				wrong.as_object().unwrap(),
				Dialect::Sqlite
			)
			.is_err()
		);
		assert_eq!(hex("0"), None);
		assert_eq!(hex("zz"), None);
	}
}
