//! Shapes: a struct saying what to fetch about a row and the rows linked to
//! it, fetched in one statement. `#[derive(db::Shape)]` describes one.
//!
//! Each nested field becomes a correlated subquery in the select list, in
//! its own scope, so the linked table's policy and soft deletes apply to it
//! exactly as they would to a query of that table. Nested rows come back as
//! JSON arrays, one element per field in declaration order, and are decoded
//! by each field's column type.
//!
//! ```text
//! SELECT lk_shape_0.handle,
//!        (SELECT count(*) FROM posts AS lk_shape_1 WHERE lk_shape_1.author_id = lk_shape_0.id AND …),
//!        (SELECT json_agg(lk_value ORDER BY lk_rank) FROM (SELECT json_build_array(…) AS lk_value,
//!                row_number() OVER (ORDER BY …) AS lk_rank FROM posts AS lk_shape_1 WHERE … LIMIT 3) AS …)
//! FROM users AS lk_shape_0 WHERE …
//! ```

use std::marker::PhantomData;

use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::TableName;
use leptoskit_db_schema::quote_identifier;

use crate::Action;
use crate::Col;
use crate::Column;
use crate::Connection;
use crate::DbError;
use crate::DecodeError;
use crate::Expr;
use crate::HasId;
use crate::Link;
use crate::Order;
use crate::Query;
use crate::Result;
use crate::Table;
use crate::Value;
use crate::policy::Access;
use crate::sql::Sql;
use crate::sql::Statement;

/// What to fetch about a row of [`Shape::From`], and about the rows linked
/// to it. `#[derive(db::Shape)]` implements it:
///
/// ```ignore
/// #[derive(db::Shape, Serialize)]
/// #[db(from = User, public)]
/// pub struct ProfileCard {
/// 	pub handle: Handle,
/// 	#[db(count(posts))]
/// 	pub post_count: i64,
/// 	#[db(via = posts, order_by = |post| post.created_at.desc(), limit = 3)]
/// 	pub latest: Vec<PostTitle>,
/// }
///
/// let card: ProfileCard = User::query().filter(|u| u.handle.eq(handle)).shape().one(&db).await?;
/// ```
pub trait Shape: Sized + 'static {
	/// The table each shaped row comes from.
	type From: Table;

	/// The fields to fetch, in declaration order.
	#[doc(hidden)]
	fn selection() -> Vec<Selected>;

	/// A shaped row from values in [`Shape::selection`] order.
	#[doc(hidden)]
	fn from_values(values: Vec<Value>) -> Result<Self>;
}

/// A shape that names no `private` or `secret` column, so it may be sent to
/// a client: `#[db(…, public)]` on the derive checks it and implements this.
pub trait PublicShape: Shape {}

/// A query that fetches shapes: [`Query::shape`] starts one.
#[must_use = "a query does nothing until it runs"]
pub struct Shaped<S: Shape> {
	query: Query<S::From>,
	shape: PhantomData<fn() -> S>,
}

impl<S: Shape> Shaped<S> {
	pub(crate) fn new(query: Query<S::From>) -> Self {
		Self {
			query,
			shape: PhantomData,
		}
	}

	/// Every matching row's shape.
	pub async fn all(self, connection: &impl Connection) -> Result<Vec<S>> {
		let statement = self.statement(connection)?;
		connection
			.fetch(statement)
			.await?
			.into_iter()
			.map(S::from_values)
			.collect()
	}

	/// The first matching row's shape, if any.
	pub async fn first(self, connection: &impl Connection) -> Result<Option<S>> {
		let first = Self::new(self.query.limit(1));
		Ok(first.all(connection).await?.into_iter().next())
	}

	/// The first matching row's shape, or [`DbError::NotFound`].
	pub async fn one(self, connection: &impl Connection) -> Result<S> {
		self.first(connection).await?.ok_or_else(|| {
			DbError::NotFound {
				table: <S::From as Table>::flat_name(),
			}
		})
	}

	/// The statement the query runs on `connection`, for tests and for
	/// reviewing the SQL a shape compiles to.
	#[doc(hidden)]
	pub fn statement(&self, connection: &impl Connection) -> Result<Statement> {
		let mut sql = Sql::new(connection.dialect());
		let (table, alias) = sql.open(&<S::From as Table>::table_name(), "shape");
		sql.push("SELECT ");
		write_fields(
			&mut sql,
			connection.access(),
			&S::selection(),
			&alias,
			false,
		)?;
		sql.push(" FROM ").push(&table).push(" AS ").push(&alias);
		self.query
			.write_filter(&mut sql, connection, Action::Select)?;
		self.query.write_tail(&mut sql);
		sql.leave();
		Ok(sql.finish())
	}
}

/// Types a value is about without holding any.
type Marker<T> = PhantomData<fn() -> T>;

/// How many rows a relation reaches from one row: exactly one, at most one,
/// or any number.
#[doc(hidden)]
pub struct One;

#[doc(hidden)]
pub struct MaybeOne;

#[doc(hidden)]
pub struct Many;

/// A way from a row of `From` to rows of `To`: a link column, a backlink,
/// or a link table. `#[db::table]` and `#[db::link]` generate one per link,
/// for shapes to follow.
#[doc(hidden)]
pub struct Relation<From, To, Cardinality> {
	definition: RelationDefinition,
	types: Marker<(From, To, Cardinality)>,
}

struct RelationDefinition {
	target: Target,
	kind: RelationKind,
}

/// A table a shape reads, with what decides which of its rows it sees.
struct Target {
	table: TableName,
	key: &'static [&'static str],
	deleted_at: Option<&'static str>,
	policy: fn(&Access, Action) -> Option<Expr>,
}

impl Target {
	fn of<T: Table>() -> Self {
		Self {
			table: T::table_name(),
			key: T::KEY,
			deleted_at: T::DELETED_AT,
			policy: crate::policy::condition::<T>,
		}
	}
}

enum RelationKind {
	/// The outer row's `column` holds the target's key.
	Forward { column: &'static str },
	/// The target's `column` holds the outer row's `key`.
	Backward {
		column: &'static str,
		key: &'static str,
	},
	/// A link table's rows pair the outer row's `key`, in `near`, with the
	/// target's key, in `far`.
	Through {
		link: Target,
		near: &'static str,
		far: &'static str,
		key: &'static str,
	},
}

impl<From: Table, To: HasId> Relation<From, To, One> {
	/// A required link: `From`'s `column` holds a `To` key.
	pub fn forward(column: &'static str) -> Self {
		Self::with(Target::of::<To>(), RelationKind::Forward { column })
	}
}

impl<From: Table, To: HasId> Relation<From, To, MaybeOne> {
	/// An optional link: `From`'s `column` may hold a `To` key.
	pub fn forward_optional(column: &'static str) -> Self {
		Self::with(Target::of::<To>(), RelationKind::Forward { column })
	}
}

impl<From: HasId, To: Table> Relation<From, To, Many> {
	/// A backlink: the `To` rows whose `column` holds a `From` key.
	pub fn backward(column: &'static str) -> Self {
		Self::with(
			Target::of::<To>(),
			RelationKind::Backward {
				column,
				key: From::KEY[0],
			},
		)
	}

	/// A link table `J`: the `To` rows paired with a `From` row, through the
	/// link's `near` and `far` columns.
	pub fn through<J: Link>(near: &'static str, far: &'static str) -> Self
	where
		To: HasId,
	{
		Self::with(
			Target::of::<To>(),
			RelationKind::Through {
				link: Target::of::<J>(),
				near,
				far,
				key: From::KEY[0],
			},
		)
	}
}

impl<From, To: Table, Cardinality> Relation<From, To, Cardinality> {
	fn with(target: Target, kind: RelationKind) -> Self {
		Self {
			definition: RelationDefinition { target, kind },
			types: PhantomData,
		}
	}

	/// How many rows the relation reaches.
	pub fn count(self) -> Related<To> {
		Related::new(self.definition, Fetch::Count)
	}

	/// The first row the relation reaches, as shape `S`, if there is one.
	pub fn optional<S: Shape<From = To>>(self) -> Related<To> {
		Related::new(
			self.definition,
			Fetch::Rows {
				selection: S::selection,
				arity: Arity::Optional,
			},
		)
	}
}

impl<From, To: Table> Relation<From, To, One> {
	/// The row a required link points at, as shape `S`.
	pub fn one<S: Shape<From = To>>(self) -> Related<To> {
		Related::new(
			self.definition,
			Fetch::Rows {
				selection: S::selection,
				arity: Arity::One,
			},
		)
	}
}

impl<From, To: Table> Relation<From, To, Many> {
	/// Every row the relation reaches, each as shape `S`.
	pub fn many<S: Shape<From = To>>(self) -> Related<To> {
		Related::new(
			self.definition,
			Fetch::Rows {
				selection: S::selection,
				arity: Arity::Many,
			},
		)
	}
}

/// A shape's field that reads linked rows: their count, or their shapes.
/// Narrow it with [`Related::filter`], and order and limit nested rows.
#[doc(hidden)]
#[must_use]
pub struct Related<T> {
	definition: RelationDefinition,
	fetch: Fetch,
	filter: Option<Expr>,
	order: Vec<Order>,
	limit: Option<u64>,
	offset: Option<u64>,
	with_deleted: bool,
	table: PhantomData<fn() -> T>,
}

enum Fetch {
	Count,
	Rows {
		selection: fn() -> Vec<Selected>,
		arity: Arity,
	},
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arity {
	One,
	Optional,
	Many,
}

impl<T: Table> Related<T> {
	fn new(definition: RelationDefinition, fetch: Fetch) -> Self {
		Self {
			definition,
			fetch,
			filter: None,
			order: Vec::new(),
			limit: None,
			offset: None,
			with_deleted: false,
			table: PhantomData,
		}
	}

	/// Reads soft-deleted rows too, as [`Query::with_deleted`] does. A link
	/// table's own soft-deleted rows still don't link anything.
	pub fn with_deleted(mut self) -> Self {
		self.with_deleted = true;
		self
	}

	/// Keeps only linked rows matching `condition`.
	pub fn filter(mut self, condition: impl FnOnce(&T::Columns) -> Expr) -> Self {
		let condition = condition(T::columns());
		self.filter = Some(match self.filter.take() {
			Some(filter) => filter & condition,
			None => condition,
		});
		self
	}

	/// Orders nested rows; later orderings break ties in earlier ones, and
	/// the key breaks any that remain.
	pub fn order_by(mut self, order: impl FnOnce(&T::Columns) -> Order) -> Self {
		self.order.push(order(T::columns()));
		self
	}

	/// At most this many nested rows.
	pub fn limit(mut self, limit: u64) -> Self {
		self.limit = Some(limit);
		self
	}

	/// Skips this many nested rows.
	pub fn offset(mut self, offset: u64) -> Self {
		self.offset = Some(offset);
		self
	}
}

/// One field of a shape, as [`Shape::selection`] lists it.
#[doc(hidden)]
pub struct Selected(Field);

enum Field {
	Column { name: &'static str, ty: ColumnType },
	Related(Box<RelatedField>),
}

/// A [`Related`] without its table type, once it's part of a selection.
struct RelatedField {
	definition: RelationDefinition,
	fetch: Fetch,
	filter: Option<Expr>,
	order: Vec<Order>,
	limit: Option<u64>,
	offset: Option<u64>,
	with_deleted: bool,
}

impl Selected {
	/// A column of the shape's table.
	pub fn column<T: Table, V: Column>(column: Col<T, V>) -> Self {
		Self(Field::Column {
			name: column.name(),
			ty: V::column_type(),
		})
	}

	/// What a nested value decodes from: a column's type, a count, or JSON.
	fn kind(&self) -> Kind {
		match &self.0 {
			Field::Column { ty, .. } => Kind::Column(ty.clone()),
			Field::Related(related) if matches!(related.fetch, Fetch::Count) => Kind::Count,
			Field::Related(_) => Kind::Nested,
		}
	}
}

impl<T> From<Related<T>> for Selected {
	fn from(related: Related<T>) -> Self {
		Self(Field::Related(Box::new(RelatedField {
			definition: related.definition,
			fetch: related.fetch,
			filter: related.filter,
			order: related.order,
			limit: related.limit,
			offset: related.offset,
			with_deleted: related.with_deleted,
		})))
	}
}

enum Kind {
	Column(ColumnType),
	Count,
	Nested,
}

/// Writes the select list of a shape whose row is `alias`. Inside a nested
/// row (`in_json`), every value must be one JSON can hold.
fn write_fields(
	sql: &mut Sql,
	access: &Access,
	fields: &[Selected],
	alias: &str,
	in_json: bool,
) -> Result<()> {
	for (index, field) in fields.iter().enumerate() {
		if index > 0 {
			sql.push(", ");
		}

		match &field.0 {
			Field::Column { name, ty } if in_json => json_column(sql, alias, name, ty),
			Field::Column { name, .. } => {
				sql.push(alias).push(".").identifier(name);
			}
			Field::Related(related) => {
				// SQLite's subqueries return JSON as text, which a JSON array
				// would hold as a string unless it's parsed back.
				let reparse = in_json
					&& sql.dialect() == Dialect::Sqlite
					&& !matches!(related.fetch, Fetch::Count);

				if reparse {
					sql.push("json(");
				}

				write_related(sql, access, related, alias)?;

				if reparse {
					sql.push(")");
				}
			}
		}
	}

	Ok(())
}

/// A column as a value JSON can hold: SQLite keeps UUIDs and bytes as blobs,
/// and Postgres would write bytes in its escape format.
fn json_column(sql: &mut Sql, alias: &str, name: &str, ty: &ColumnType) {
	let column = format!("{alias}.{}", quote_identifier(name));

	match (sql.dialect(), ty) {
		(Dialect::Postgres, ColumnType::Bytes) => {
			sql.push("encode(").push(&column).push(", 'hex')");
		}
		(Dialect::Sqlite, ColumnType::Uuid | ColumnType::Bytes) => {
			// `hex(NULL)` is an empty string, so `NULL` stays `NULL` first.
			sql.push("CASE WHEN ")
				.push(&column)
				.push(" IS NULL THEN NULL ELSE lower(hex(")
				.push(&column)
				.push(")) END");
		}
		(Dialect::Sqlite, ColumnType::Json) => {
			sql.push("json(").push(&column).push(")");
		}
		_ => {
			sql.push(&column);
		}
	}
}

/// One related field: a count, a nested row, or an array of nested rows,
/// as a subquery correlated with the row `outer` names.
fn write_related(
	sql: &mut Sql,
	access: &Access,
	related: &RelatedField,
	outer: &str,
) -> Result<()> {
	let (table, alias) = sql.open(&related.definition.target.table, "shape");
	let written = write_subquery(sql, access, related, outer, &table, &alias);
	sql.leave();
	written
}

/// The subquery [`write_related`] opened a scope for: `table AS alias`.
fn write_subquery(
	sql: &mut Sql,
	access: &Access,
	related: &RelatedField,
	outer: &str,
	table: &str,
	alias: &str,
) -> Result<()> {
	match &related.fetch {
		Fetch::Count => {
			sql.push("(SELECT count(*) FROM ")
				.push(table)
				.push(" AS ")
				.push(alias);
			write_related_where(sql, access, related, outer, alias)?;
			sql.push(")");
		}
		Fetch::Rows {
			selection,
			arity: Arity::Many,
		} => {
			let rows = quote_identifier(&format!("lk_rows_{}", sql.depth()));
			sql.push(match sql.dialect() {
				Dialect::Postgres => {
					"(SELECT coalesce(json_agg(\"lk_value\" ORDER BY \"lk_rank\"), '[]'::json)"
				}
				// A subquery's column loses its JSON subtype, so each row is
				// parsed back before it joins the array.
				Dialect::Sqlite => {
					"(SELECT json_group_array(json(\"lk_value\") ORDER BY \"lk_rank\")"
				}
			});
			sql.push(" FROM (SELECT ");
			write_row(sql, access, &selection(), alias)?;
			sql.push(" AS \"lk_value\", row_number() OVER (");
			write_order(sql, related);
			sql.push(") AS \"lk_rank\" FROM ")
				.push(table)
				.push(" AS ")
				.push(alias);
			write_related_where(sql, access, related, outer, alias)?;
			sql.push(" ");
			write_order(sql, related);
			crate::query::write_order(sql, &[], related.limit, related.offset);
			sql.push(") AS ").push(&rows).push(")");
		}
		Fetch::Rows { selection, .. } => {
			sql.push("(SELECT ");
			write_row(sql, access, &selection(), alias)?;
			sql.push(" FROM ").push(table).push(" AS ").push(alias);
			write_related_where(sql, access, related, outer, alias)?;
			sql.push(" ");
			write_order(sql, related);
			crate::query::write_order(sql, &[], Some(1), related.offset);
			sql.push(")");
		}
	}

	Ok(())
}

/// A nested row: a JSON array of its fields.
fn write_row(sql: &mut Sql, access: &Access, fields: &[Selected], alias: &str) -> Result<()> {
	sql.push(match sql.dialect() {
		Dialect::Postgres => "json_build_array(",
		Dialect::Sqlite => "json_array(",
	});
	write_fields(sql, access, fields, alias, true)?;
	sql.push(")");
	Ok(())
}

/// `ORDER BY` the field's orderings, then the target's key, so nested rows
/// always come in the same order.
fn write_order(sql: &mut Sql, related: &RelatedField) {
	sql.push("ORDER BY ");

	for order in &related.order {
		order.write(sql);
		sql.push(", ");
	}

	for (index, key) in related.definition.target.key.iter().enumerate() {
		if index > 0 {
			sql.push(", ");
		}

		sql.identifier(key).push(" ASC");
	}
}

/// `WHERE` the target row is the outer row's, the field's filter matches,
/// the request may read it, and it isn't soft-deleted.
fn write_related_where(
	sql: &mut Sql,
	access: &Access,
	related: &RelatedField,
	outer: &str,
	alias: &str,
) -> Result<()> {
	let target = &related.definition.target;
	sql.push(" WHERE ");

	match &related.definition.kind {
		RelationKind::Forward { column } => {
			sql.push(alias)
				.push(".")
				.identifier(target.key[0])
				.push(" = ")
				.push(outer)
				.push(".")
				.identifier(column);
		}
		RelationKind::Backward { column, key } => {
			sql.push(alias)
				.push(".")
				.identifier(column)
				.push(" = ")
				.push(outer)
				.push(".")
				.identifier(key);
		}
		RelationKind::Through {
			link,
			near,
			far,
			key,
		} => {
			sql.push(alias)
				.push(".")
				.identifier(target.key[0])
				.push(" IN (SELECT ");
			let (table, links) = sql.open(&link.table, "link");
			sql.push(&links)
				.push(".")
				.identifier(far)
				.push(" FROM ")
				.push(&table)
				.push(" AS ")
				.push(&links)
				.push(" WHERE ")
				.push(&links)
				.push(".")
				.identifier(near)
				.push(" = ")
				.push(outer)
				.push(".")
				.identifier(key);
			let written = write_visible(sql, access, link, false);
			sql.leave();
			written?;
			sql.push(")");
		}
	}

	if let Some(filter) = &related.filter {
		sql.push(" AND (");
		filter.write(sql)?;
		sql.push(")");
	}

	write_visible(sql, access, target, related.with_deleted)
}

/// `AND` what the request may read of the innermost scope's table, and not
/// soft-deleted unless `with_deleted`.
fn write_visible(
	sql: &mut Sql,
	access: &Access,
	target: &Target,
	with_deleted: bool,
) -> Result<()> {
	if let Some(policy) = (target.policy)(access, Action::Select) {
		sql.push(" AND (");
		sql.isolated(|sql| policy.write(sql))?;
		sql.push(")");
	}

	if let Some(column) = target.deleted_at.filter(|_| !with_deleted) {
		sql.push(" AND ").identifier(column).push(" IS NULL");
	}

	Ok(())
}

/// Decodes the next value of a shaped row as a column or a count.
#[doc(hidden)]
pub fn decode_column<V: Column>(
	values: &mut impl Iterator<Item = Value>,
	shape: &str,
	field: &str,
) -> Result<V> {
	let value = next(values, shape, field)?;
	V::from_value(value).map_err(|error| DbError::Decode(error.at(shape, field)))
}

/// Decodes the next value as the row a required link points at. A row the
/// request may not see, or a soft-deleted one, is not found, as it is to
/// the link's accessor.
#[doc(hidden)]
pub fn decode_one<S: Shape>(
	values: &mut impl Iterator<Item = Value>,
	shape: &str,
	field: &str,
) -> Result<S> {
	decode_optional(values, shape, field)?.ok_or_else(|| {
		DbError::NotFound {
			table: <S::From as Table>::flat_name(),
		}
	})
}

/// Decodes the next value as at most one nested row.
#[doc(hidden)]
pub fn decode_optional<S: Shape>(
	values: &mut impl Iterator<Item = Value>,
	shape: &str,
	field: &str,
) -> Result<Option<S>> {
	match json(next(values, shape, field)?, shape, field)? {
		serde_json::Value::Null => Ok(None),
		row => nested::<S>(row, &kinds::<S>(), shape, field).map(Some),
	}
}

/// Decodes the next value as an array of nested rows.
#[doc(hidden)]
pub fn decode_many<S: Shape>(
	values: &mut impl Iterator<Item = Value>,
	shape: &str,
	field: &str,
) -> Result<Vec<S>> {
	match json(next(values, shape, field)?, shape, field)? {
		serde_json::Value::Array(rows) => {
			let kinds = kinds::<S>();
			rows.into_iter()
				.map(|row| nested::<S>(row, &kinds, shape, field))
				.collect()
		}
		other => {
			Err(located(
				format!("expected an array, found {other}"),
				shape,
				field,
			))
		}
	}
}

fn next(values: &mut impl Iterator<Item = Value>, shape: &str, field: &str) -> Result<Value> {
	values
		.next()
		.ok_or_else(|| located("the row has too few values", shape, field))
}

fn located(message: impl std::fmt::Display, shape: &str, field: &str) -> DbError {
	DbError::Decode(DecodeError::custom(message).at(shape, field))
}

/// A nested value as JSON: Postgres returns `json`, and SQLite text.
fn json(value: Value, shape: &str, field: &str) -> Result<serde_json::Value> {
	match value {
		Value::Json(json) => Ok(json),
		Value::Text(text) => {
			serde_json::from_str(&text).map_err(|error| located(error, shape, field))
		}
		Value::Null => Ok(serde_json::Value::Null),
		other => {
			Err(DbError::Decode(
				DecodeError::unexpected("json", &other).at(shape, field),
			))
		}
	}
}

/// What each of a shape's fields decodes from.
fn kinds<S: Shape>() -> Vec<Kind> {
	S::selection().iter().map(Selected::kind).collect()
}

/// A nested row, from its JSON array of field values.
fn nested<S: Shape>(row: serde_json::Value, kinds: &[Kind], shape: &str, field: &str) -> Result<S> {
	let serde_json::Value::Array(elements) = row else {
		return Err(located(
			format!("expected a nested row, found {row}"),
			shape,
			field,
		));
	};
	if elements.len() != kinds.len() {
		return Err(located(
			format!(
				"a nested row has {} values for {} fields",
				elements.len(),
				kinds.len()
			),
			shape,
			field,
		));
	}

	let values = kinds
		.iter()
		.zip(elements)
		.map(|(kind, element)| {
			from_json(kind, element).map_err(|error| DbError::Decode(error.at(shape, field)))
		})
		.collect::<Result<Vec<_>>>()?;
	S::from_values(values)
}

/// A nested value, as the database value its column would have held.
fn from_json(kind: &Kind, json: serde_json::Value) -> std::result::Result<Value, DecodeError> {
	use serde_json::Value as Json;

	let ty = match kind {
		Kind::Nested => return Ok(Value::Json(json)),
		Kind::Count => &ColumnType::Int64,
		Kind::Column(ty) => ty,
	};
	let mismatch = |json: &Json| DecodeError::custom(format!("expected {ty}, found {json}"));

	Ok(match (ty, json) {
		(_, Json::Null) => Value::Null,
		(ColumnType::Bool, Json::Bool(value)) => Value::Bool(value),
		(
			ColumnType::Bool | ColumnType::Int16 | ColumnType::Int32 | ColumnType::Int64,
			Json::Number(number),
		) => {
			Value::Int(
				number
					.as_i64()
					.ok_or_else(|| mismatch(&Json::Number(number)))?,
			)
		}
		(ColumnType::Float32 | ColumnType::Float64, Json::Number(number)) => {
			Value::Float(
				number
					.as_f64()
					.ok_or_else(|| mismatch(&Json::Number(number)))?,
			)
		}
		// Postgres writes the floats JSON has no numbers for as text.
		(ColumnType::Float32 | ColumnType::Float64, Json::String(text)) => {
			Value::Float(text.parse().map_err(|_| mismatch(&Json::String(text)))?)
		}
		(
			ColumnType::Text { .. } | ColumnType::Timestamp | ColumnType::Date,
			Json::String(text),
		) => Value::Text(text),
		(ColumnType::Uuid, Json::String(text)) => {
			Value::Uuid(uuid::Uuid::parse_str(&text).map_err(DecodeError::custom)?)
		}
		(ColumnType::Bytes, Json::String(text)) => Value::Bytes(from_hex(&text)?),
		(ColumnType::Json, json) => Value::Json(json),
		(ColumnType::Decimal { .. } | ColumnType::Array { .. }, _) => {
			return Err(DecodeError::custom(format!(
				"a nested shape can't hold a {ty} column yet; select it at the top level"
			)));
		}
		(_, json) => return Err(mismatch(&json)),
	})
}

fn from_hex(text: &str) -> std::result::Result<Vec<u8>, DecodeError> {
	let digit = |byte: u8| {
		char::from(byte)
			.to_digit(16)
			.and_then(|digit| u8::try_from(digit).ok())
			.ok_or_else(|| DecodeError::custom(format!("`{text}` isn't hexadecimal")))
	};

	if text.len() % 2 != 0 {
		return Err(DecodeError::custom(format!("`{text}` isn't hexadecimal")));
	}

	text.as_bytes()
		.chunks(2)
		.map(|pair| Ok(digit(pair[0])? << 4 | digit(pair[1])?))
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A note, to shape in the decoding tests.
	#[crate::table(name = "notes")]
	pub struct Note {
		pub id: crate::Id<Self>,
		pub body: String,
	}

	#[derive(crate::Shape, Debug, PartialEq)]
	#[db(from = Note)]
	pub struct Body {
		pub body: String,
	}

	fn values(values: Vec<Value>) -> std::vec::IntoIter<Value> {
		values.into_iter()
	}

	fn message(error: &DbError) -> String {
		match error {
			DbError::Decode(error) => error.to_string(),
			other => panic!("expected a decode error, found {other:?}"),
		}
	}

	#[test]
	fn nested_rows_decode_from_either_databases_json() {
		let many = |value: Value| decode_many::<Body>(&mut values(vec![value]), "Card", "bodies");
		let body = |text: &str| Body { body: text.into() };

		// Postgres returns `json`, SQLite text.
		assert_eq!(
			many(Value::Json(serde_json::json!([["a"], ["b"]]))).unwrap(),
			[body("a"), body("b")]
		);
		assert_eq!(many(Value::Text("[[\"a\"]]".into())).unwrap(), [body("a")]);
		assert_eq!(
			decode_optional::<Body>(&mut values(vec![Value::Null]), "Card", "body").unwrap(),
			None
		);

		// A required row that isn't there is not found, as its link would be.
		let missing = decode_one::<Body>(&mut values(vec![Value::Null]), "Card", "body");
		assert!(missing.unwrap_err().is_not_found());
	}

	#[test]
	fn malformed_nested_rows_name_the_field() {
		let many = |value: Value| {
			message(&decode_many::<Body>(&mut values(vec![value]), "Card", "bodies").unwrap_err())
		};

		assert_eq!(
			many(Value::Json(serde_json::json!({}))),
			"Card.bodies: expected an array, found {}"
		);
		assert_eq!(
			many(Value::Json(serde_json::json!([3]))),
			"Card.bodies: expected a nested row, found 3"
		);
		assert_eq!(
			many(Value::Json(serde_json::json!([["a", "b"]]))),
			"Card.bodies: a nested row has 2 values for 1 fields"
		);
		assert_eq!(
			many(Value::Json(serde_json::json!([[3]]))),
			"Card.bodies: expected text, found 3"
		);
		assert_eq!(
			many(Value::Int(1)),
			"Card.bodies: expected json, found integer"
		);
		assert!(many(Value::Text("[".into())).starts_with("Card.bodies: "));

		let empty = decode_many::<Body>(&mut values(Vec::new()), "Card", "bodies").unwrap_err();
		assert_eq!(message(&empty), "Card.bodies: the row has too few values");
		let wrong = decode_column::<i64>(&mut values(vec![Value::Text("x".into())]), "Card", "n");
		assert_eq!(
			message(&wrong.unwrap_err()),
			"Card.n: expected integer, found text"
		);
	}

	#[test]
	fn nested_values_decode_by_column_type() {
		let decode = |ty: ColumnType, json: serde_json::Value| from_json(&Kind::Column(ty), json);

		assert_eq!(
			decode(ColumnType::Bool, serde_json::json!(1)).unwrap(),
			Value::Int(1)
		);
		assert_eq!(
			decode(ColumnType::Float64, serde_json::json!("NaN"))
				.unwrap()
				.kind(),
			"float"
		);
		assert_eq!(
			decode(ColumnType::Bytes, serde_json::json!("00ff")).unwrap(),
			Value::Bytes(vec![0, 255])
		);
		let id = uuid::Uuid::now_v7();
		assert_eq!(
			decode(ColumnType::Uuid, serde_json::json!(id.simple().to_string())).unwrap(),
			Value::Uuid(id)
		);
		assert_eq!(
			decode(ColumnType::Int32, serde_json::Value::Null).unwrap(),
			Value::Null
		);
		assert_eq!(
			from_json(&Kind::Count, serde_json::json!(3)).unwrap(),
			Value::Int(3)
		);

		for (ty, json) in [
			(ColumnType::Int64, serde_json::json!("3")),
			(ColumnType::Int64, serde_json::json!(1.5)),
			(ColumnType::Text { max_len: None }, serde_json::json!(3)),
			(ColumnType::Bytes, serde_json::json!("0")),
			(ColumnType::Bytes, serde_json::json!("zz")),
			(ColumnType::Uuid, serde_json::json!("nope")),
			(
				ColumnType::Decimal {
					precision: None,
					scale: None,
				},
				serde_json::json!(1),
			),
		] {
			assert!(
				decode(ty.clone(), json.clone()).is_err(),
				"{ty} from {json}"
			);
		}
	}
}
