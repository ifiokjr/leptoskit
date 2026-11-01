//! What the studio does to one table, whatever its Rust type: each table
//! registers these functions, instantiated for it, so every read and write
//! goes through the table's own queries, policies, hooks, and audit log.

use std::future::Future;
use std::pin::Pin;

use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::Dialect;

use crate::Action;
use crate::Db;
use crate::DbError;
use crate::Expr;
use crate::Order;
use crate::Query;
use crate::Result;
use crate::Statement;
use crate::Table;
use crate::Value;
use crate::expr::Comparison;
use crate::policy::Access;
use crate::sql::Sql;

/// A future the studio's handlers can hold across awaits.
pub(crate) type Boxed<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// A future that holds a row across awaits, which needn't be `Send`: the
/// studio runs it on a thread of its own.
pub(crate) type Local<T> = Pin<Box<dyn Future<Output = Result<T>>>>;

/// What a viewer may do to a table, action by action: the statement the
/// database layer adds, or `None` when nothing restricts it.
pub(crate) type Rules = Vec<(Action, Option<Statement>)>;

/// One table's operations, as plain function pointers.
#[derive(Clone, Copy)]
pub(crate) struct TableOps {
	pub(crate) read: for<'a> fn(&'a Db, &'a Browse) -> Boxed<'a, Rows>,
	pub(crate) edit: fn(Db, Edit) -> Local<()>,
	pub(crate) remove: fn(Db, Vec<String>) -> Local<()>,
	pub(crate) rules: fn(&Access, Dialect) -> Result<Rules>,
}

impl TableOps {
	pub(crate) const fn of<T: Table>() -> Self {
		Self {
			read: read::<T>,
			edit: edit::<T>,
			remove: remove::<T>,
			rules: rules::<T>,
		}
	}
}

/// A comparison a filter makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
	Equal,
	NotEqual,
	Greater,
	Less,
	Contains,
	IsNull,
	IsNotNull,
}

impl Op {
	pub(crate) const ALL: [Self; 7] = [
		Self::Equal,
		Self::NotEqual,
		Self::Greater,
		Self::Less,
		Self::Contains,
		Self::IsNull,
		Self::IsNotNull,
	];

	pub(crate) fn name(self) -> &'static str {
		match self {
			Self::Equal => "eq",
			Self::NotEqual => "ne",
			Self::Greater => "gt",
			Self::Less => "lt",
			Self::Contains => "contains",
			Self::IsNull => "null",
			Self::IsNotNull => "not_null",
		}
	}

	pub(crate) fn label(self) -> &'static str {
		match self {
			Self::Equal => "=",
			Self::NotEqual => "≠",
			Self::Greater => ">",
			Self::Less => "<",
			Self::Contains => "contains",
			Self::IsNull => "is null",
			Self::IsNotNull => "is not null",
		}
	}

	pub(crate) fn parse(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|op| op.name() == name)
	}
}

/// One filter on a column.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Filter {
	pub(crate) column: String,
	pub(crate) op: Op,
	pub(crate) value: String,
}

/// What a table page asks for.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Browse {
	pub(crate) filter: Option<Filter>,
	pub(crate) order: Option<(String, bool)>,
	pub(crate) limit: u64,
	pub(crate) offset: u64,
	pub(crate) with_deleted: bool,
}

/// A page of rows, as text, and how many rows match in all.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rows {
	pub(crate) columns: Vec<&'static str>,
	pub(crate) key: Vec<usize>,
	pub(crate) cells: Vec<Vec<Cell>>,
	pub(crate) total: u64,
}

/// One value, as the studio shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Cell {
	Null,
	Secret,
	Text(String),
}

impl Cell {
	pub(crate) fn text(&self) -> &str {
		match self {
			Self::Null => "NULL",
			Self::Secret => "‹secret›",
			Self::Text(text) => text,
		}
	}
}

/// A change to one column of one row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Edit {
	/// The row's key, one text per key column.
	pub(crate) key: Vec<String>,
	pub(crate) column: String,
	/// The new value, or `None` for `NULL`.
	pub(crate) value: Option<String>,
}

fn invalid(message: impl Into<String>) -> DbError {
	crate::expr::invalid(&message.into())
}

/// The table's own name for a column, or an error naming the table's
/// columns.
fn column_of<T: Table>(name: &str) -> Result<&'static str> {
	T::COLUMNS
		.iter()
		.copied()
		.find(|column| *column == name)
		.ok_or_else(|| {
			invalid(format!(
				"`{}` has no column `{name}`; it has {}",
				T::flat_name(),
				T::COLUMNS.join(", ")
			))
		})
}

/// A column's type and whether it takes `NULL`.
fn column_type<T: Table>(column: &str) -> Result<(ColumnType, bool)> {
	T::definition()
		.columns
		.into_iter()
		.find(|candidate| candidate.name == column)
		.map(|column| (column.ty, column.nullable))
		.ok_or_else(|| invalid(format!("`{}` has no column `{column}`", T::flat_name())))
}

/// The value `text` means in a column of type `ty`.
pub(crate) fn parse_value(ty: &ColumnType, text: &str) -> std::result::Result<Value, String> {
	let integer = |min: i64, max: i64| {
		text.trim()
			.parse::<i64>()
			.map_err(|error| format!("`{text}` isn't a whole number: {error}"))
			.and_then(|number| {
				if (min..=max).contains(&number) {
					Ok(Value::Int(number))
				} else {
					Err(format!("{number} is outside {min} to {max}"))
				}
			})
	};

	match ty {
		ColumnType::Bool => {
			match text.trim() {
				"true" => Ok(Value::Bool(true)),
				"false" => Ok(Value::Bool(false)),
				other => Err(format!("`{other}` isn't `true` or `false`")),
			}
		}
		ColumnType::Int16 => integer(i16::MIN.into(), i16::MAX.into()),
		ColumnType::Int32 => integer(i32::MIN.into(), i32::MAX.into()),
		ColumnType::Int64 => integer(i64::MIN, i64::MAX),
		ColumnType::Float32 | ColumnType::Float64 => {
			text.trim()
				.parse::<f64>()
				.map(Value::Float)
				.map_err(|error| format!("`{text}` isn't a number: {error}"))
		}
		ColumnType::Text { .. } => Ok(Value::Text(text.to_string())),
		ColumnType::Uuid => {
			uuid::Uuid::parse_str(text.trim())
				.map(Value::Uuid)
				.map_err(|error| format!("`{text}` isn't a UUID: {error}"))
		}
		ColumnType::Timestamp => {
			chrono::DateTime::parse_from_rfc3339(text.trim())
				.map(|instant| Value::Timestamp(instant.to_utc()))
				.map_err(|error| format!("`{text}` isn't an RFC 3339 instant: {error}"))
		}
		ColumnType::Date => {
			text.trim()
				.parse()
				.map(Value::Date)
				.map_err(|error| format!("`{text}` isn't a date, such as 2026-10-30: {error}"))
		}
		ColumnType::Json => {
			serde_json::from_str(text)
				.map(Value::Json)
				.map_err(|error| format!("that isn't JSON: {error}"))
		}
		other => {
			Err(format!(
				"the studio can't read or write `{other:?}` values yet"
			))
		}
	}
}

/// How the studio shows a value of a column of type `ty`: a vector by its
/// type and first few values, which its bytes would otherwise hide.
pub(crate) fn display_as(ty: Option<&ColumnType>, value: &Value) -> Cell {
	let (
		Some(ColumnType::Vector {
			element,
			dimensions,
		}),
		Value::Bytes(bytes),
	) = (ty, value)
	else {
		return display(value);
	};
	let first: Vec<String> = match element {
		leptoskit_db_schema::VectorElement::Float32
		| leptoskit_db_schema::VectorElement::Float16 => {
			bytes
				.as_chunks::<4>()
				.0
				.iter()
				.take(3)
				.map(|chunk| f32::from_le_bytes(*chunk).to_string())
				.collect()
		}
		leptoskit_db_schema::VectorElement::Bit => {
			bytes
				.iter()
				.take(2)
				.map(|byte| format!("{:08b}", byte.reverse_bits()))
				.collect()
		}
		leptoskit_db_schema::VectorElement::Sparse => Vec::new(),
	};
	let more = if first.is_empty() { "…" } else { ", …" };

	Cell::Text(format!(
		"{}({dimensions}) [{}{more}]",
		element.postgres_name(),
		first.join(", ")
	))
}

/// How the studio shows a value.
pub(crate) fn display(value: &Value) -> Cell {
	match value {
		Value::Null => Cell::Null,
		Value::Bool(value) => Cell::Text(value.to_string()),
		Value::Int(value) => Cell::Text(value.to_string()),
		Value::Float(value) => Cell::Text(value.to_string()),
		Value::Text(text) => Cell::Text(text.clone()),
		Value::Bytes(bytes) => {
			// SQLite stores UUIDs as 16 bytes.
			match uuid::Uuid::from_slice(bytes) {
				Ok(uuid) if bytes.len() == 16 => Cell::Text(uuid.to_string()),
				_ => Cell::Text(format!("{} bytes", bytes.len())),
			}
		}
		Value::Uuid(uuid) => Cell::Text(uuid.to_string()),
		Value::Timestamp(instant) => Cell::Text(instant.to_rfc3339()),
		Value::Date(date) => Cell::Text(date.to_string()),
		Value::Json(json) => Cell::Text(json.to_string()),
	}
}

/// The condition a filter makes on `T`.
fn filter_expr<T: Table>(filter: &Filter) -> Result<Expr> {
	let column = column_of::<T>(&filter.column)?;

	let comparison = match filter.op {
		Op::IsNull | Op::IsNotNull => {
			return Ok(Expr::Null {
				column,
				negated: filter.op == Op::IsNotNull,
			});
		}
		Op::Contains => {
			return Ok(Expr::Contains {
				column,
				needle: filter.value.clone(),
			});
		}
		Op::Equal => Comparison::Equal,
		Op::NotEqual => Comparison::NotEqual,
		Op::Greater => Comparison::Greater,
		Op::Less => Comparison::Less,
	};
	let (ty, _) = column_type::<T>(column)?;
	let value = parse_value(&ty, &filter.value).map_err(invalid)?;

	Ok(Expr::Compare {
		column,
		comparison,
		value,
	})
}

/// The query a page reads: its filter, and soft-deleted rows when asked.
fn browse_query<T: Table>(browse: &Browse) -> Result<Query<T>> {
	let mut query = Query::<T>::new();

	if let Some(filter) = &browse.filter {
		let condition = filter_expr::<T>(filter)?;
		query = query.filter(|_| condition);
	}

	if browse.with_deleted {
		query = query.with_deleted();
	}

	Ok(query)
}

/// A page of `T`'s rows as `db` may read them, `secret` columns hidden.
fn read<'a, T: Table>(db: &'a Db, browse: &'a Browse) -> Boxed<'a, Rows> {
	Box::pin(async move {
		let total = browse_query::<T>(browse)?.count(db).await?;
		let mut query = browse_query::<T>(browse)?;
		let order = match &browse.order {
			Some((column, descending)) => Order::by(column_of::<T>(column)?, *descending),
			None => Order::by(T::KEY[0], false),
		};
		query = query
			.order_by(|_| order)
			.limit(browse.limit)
			.offset(browse.offset);

		let definition = T::definition();
		let cells = query
			.all(db)
			.await?
			.into_iter()
			.map(|row| {
				Ok(T::COLUMNS
					.iter()
					.zip(row.to_values()?)
					.map(|(column, value)| {
						if T::SECRET.contains(column) {
							Cell::Secret
						} else {
							display_as(definition.column(column).map(|column| &column.ty), &value)
						}
					})
					.collect())
			})
			.collect::<Result<Vec<Vec<Cell>>>>()?;
		let key = T::KEY
			.iter()
			.map(|key| {
				T::COLUMNS
					.iter()
					.position(|column| column == key)
					.expect("a key is a column")
			})
			.collect();

		Ok(Rows {
			columns: T::COLUMNS.to_vec(),
			key,
			cells,
			total,
		})
	})
}

/// The row of `T` with this key, as `db` may read it, soft-deleted or not.
async fn find<T: Table>(db: &Db, key: &[String]) -> Result<T> {
	if key.len() != T::KEY.len() {
		return Err(invalid(format!(
			"`{}` is keyed by {}",
			T::flat_name(),
			T::KEY.join(", ")
		)));
	}

	let mut condition = Expr::always();

	for (column, text) in T::KEY.iter().zip(key) {
		let (ty, _) = column_type::<T>(column)?;
		condition = condition
			& Expr::Compare {
				column,
				comparison: Comparison::Equal,
				value: parse_value(&ty, text).map_err(invalid)?,
			};
	}

	Query::<T>::new()
		.filter(|_| condition)
		.with_deleted()
		.one(db)
		.await
}

/// Changes one column of one row through `save`, with the table's hooks,
/// rules, policies, and audit log.
fn edit<T: Table>(db: Db, edit: Edit) -> Local<()> {
	Box::pin(async move {
		let column = column_of::<T>(&edit.column)?;

		if T::KEY.contains(&column) || T::GENERATED.contains(&column) {
			return Err(invalid(format!(
				"`{column}` can't change: it's part of the key or computed by the database"
			)));
		}

		let (ty, nullable) = column_type::<T>(column)?;
		let value = match &edit.value {
			None if nullable => Value::Null,
			None => return Err(invalid(format!("`{column}` can't be NULL"))),
			Some(text) => parse_value(&ty, text).map_err(invalid)?,
		};
		let row = find::<T>(&db, &edit.key).await?;
		let mut values = row.to_values()?;
		let position = T::COLUMNS
			.iter()
			.position(|candidate| *candidate == column)
			.expect("the column is the table's");
		values[position] = value;
		let mut row = T::from_values(values)?;
		crate::write::save(&db, &mut row).await
	})
}

/// Deletes one row through `delete`: soft-deletes it when the table can.
fn remove<T: Table>(db: Db, key: Vec<String>) -> Local<()> {
	Box::pin(async move {
		let mut row = find::<T>(&db, &key).await?;
		crate::write::delete(&db, &mut row).await
	})
}

/// What `access` may do to `T`'s rows, action by action, as SQL with its
/// values bound: `None` when nothing restricts it.
fn rules<T: Table>(access: &Access, dialect: Dialect) -> Result<Rules> {
	Action::ALL
		.iter()
		.map(|action| {
			let statement = crate::policy::condition::<T>(access, *action)
				.map(|condition| {
					let mut sql = Sql::new(dialect);
					sql.push("SELECT * FROM ").table::<T>().push(" WHERE ");
					condition.write(&mut sql)?;
					Ok::<_, DbError>(sql.finish())
				})
				.transpose()?;
			Ok((*action, statement))
		})
		.collect()
}
