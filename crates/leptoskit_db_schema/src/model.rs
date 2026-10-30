//! The schema model: what a database should contain, independent of how it
//! got there.
//!
//! The same types describe the schema exported from an app's Rust structs
//! (the target) and the snapshot committed beside each migration (the
//! baseline). Both serialize to the `schema.json` format.

use std::fmt;

use serde::Deserialize;
use serde::Serialize;

use crate::SchemaError;

/// The version of the `schema.json` format this crate reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// The SQL dialect a schema targets. A project has exactly one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dialect {
	/// PostgreSQL, the default.
	Postgres,
	/// SQLite, with `STRICT` tables.
	Sqlite,
}

impl fmt::Display for Dialect {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::Postgres => "postgres",
			Self::Sqlite => "sqlite",
		})
	}
}

/// Everything a database should contain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schema {
	/// The `schema.json` format version; always [`FORMAT_VERSION`] when
	/// written by this crate.
	pub format: u32,
	/// The dialect every table is rendered for.
	pub dialect: Dialect,
	/// The tables, sorted by name.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub tables: Vec<Table>,
	/// The views, sorted by name.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub views: Vec<View>,
}

impl Schema {
	/// A schema with no tables: the baseline before the first migration.
	pub fn empty(dialect: Dialect) -> Self {
		Self {
			format: FORMAT_VERSION,
			dialect,
			tables: Vec::new(),
			views: Vec::new(),
		}
	}

	/// A schema with these tables, sorted by name so that snapshots are
	/// deterministic.
	pub fn new(dialect: Dialect, mut tables: Vec<Table>) -> Self {
		tables.sort_by(|left, right| left.name.cmp(&right.name));

		Self {
			format: FORMAT_VERSION,
			dialect,
			tables,
			views: Vec::new(),
		}
	}

	/// The same schema with these views, sorted by name.
	#[must_use]
	pub fn with_views(mut self, mut views: Vec<View>) -> Self {
		views.sort_by(|left, right| left.name.cmp(&right.name));
		self.views = views;
		self
	}

	/// The view with this name, if there is one.
	pub fn view(&self, name: &TableName) -> Option<&View> {
		self.views.iter().find(|view| &view.name == name)
	}

	/// The table with this name, if there is one.
	pub fn table(&self, name: &TableName) -> Option<&Table> {
		self.tables.iter().find(|table| &table.name == name)
	}

	/// Reads a `schema.json` snapshot and validates it.
	pub fn from_json(json: &str) -> Result<Self, SchemaError> {
		let schema: Self = serde_json::from_str(json).map_err(SchemaError::Json)?;

		if schema.format != FORMAT_VERSION {
			return Err(SchemaError::UnsupportedFormat {
				found: schema.format,
				supported: FORMAT_VERSION,
			});
		}

		schema.validate()?;
		Ok(schema)
	}

	/// Writes the schema as a `schema.json` snapshot: pretty-printed with a
	/// trailing newline, so that diffs in review are line-oriented.
	pub fn to_json(&self) -> String {
		let mut json = serde_json::to_string_pretty(self).expect("a schema always serializes");
		json.push('\n');
		json
	}

	/// The same schema without rename hints. Snapshots are written this way:
	/// a hint describes how to reach this schema from the previous one, and
	/// means nothing once that migration exists.
	#[must_use]
	pub fn without_rename_hints(&self) -> Self {
		let mut schema = self.clone();

		for table in &mut schema.tables {
			table.renamed_from = None;

			for column in &mut table.columns {
				column.renamed_from = None;
			}
		}

		schema
	}
}

/// A table's name, with an optional namespace.
///
/// Postgres renders the namespace as a real schema (`"auth"."sessions"`);
/// SQLite has none, so it renders as a prefix (`"auth_sessions"`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableName {
	/// The namespace, such as `auth` for tables a schema pack owns.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub namespace: Option<String>,
	/// The table's own name.
	pub name: String,
}

impl TableName {
	/// A table outside any namespace.
	pub fn new(name: impl Into<String>) -> Self {
		Self {
			namespace: None,
			name: name.into(),
		}
	}

	/// A table inside a namespace.
	pub fn namespaced(namespace: impl Into<String>, name: impl Into<String>) -> Self {
		Self {
			namespace: Some(namespace.into()),
			name: name.into(),
		}
	}

	/// The name with its namespace joined by `_`: the SQLite table name, and
	/// the stem of every constraint and index name in both dialects.
	pub fn flat(&self) -> String {
		match &self.namespace {
			Some(namespace) => format!("{namespace}_{}", self.name),
			None => self.name.clone(),
		}
	}
}

impl fmt::Display for TableName {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match &self.namespace {
			Some(namespace) => write!(f, "{namespace}.{}", self.name),
			None => f.write_str(&self.name),
		}
	}
}

/// One table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
	/// The table's name.
	pub name: TableName,
	/// The table's documentation, from the struct's doc comment.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub comment: Option<String>,
	/// The columns, in declaration order.
	pub columns: Vec<Column>,
	/// The primary key's columns, in order.
	pub primary_key: Vec<String>,
	/// Indexes, including every unique constraint.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub indexes: Vec<Index>,
	/// Foreign keys.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub foreign_keys: Vec<ForeignKey>,
	/// Check constraints.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub checks: Vec<Check>,
	/// The table's previous name, when it was renamed. Only meaningful in a
	/// target schema; snapshots never contain it.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub renamed_from: Option<TableName>,
}

impl Table {
	/// A table with these columns and primary key, and nothing else.
	pub fn new(name: TableName, columns: Vec<Column>, primary_key: Vec<String>) -> Self {
		Self {
			name,
			comment: None,
			columns,
			primary_key,
			indexes: Vec::new(),
			foreign_keys: Vec::new(),
			checks: Vec::new(),
			renamed_from: None,
		}
	}

	/// The column with this name, if there is one.
	pub fn column(&self, name: &str) -> Option<&Column> {
		self.columns.iter().find(|column| column.name == name)
	}

	/// The name of the primary key constraint.
	pub fn primary_key_name(&self) -> String {
		crate::constraint_name(&self.name, &[], "pkey")
	}
}

/// One column.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
	/// The column's name: the Rust field's name.
	pub name: String,
	/// What the column stores.
	#[serde(rename = "type")]
	pub ty: ColumnType,
	/// Whether the column accepts `NULL`.
	pub nullable: bool,
	/// The value the database fills in when an insert omits the column.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub default: Option<DefaultValue>,
	/// The column's documentation, from the field's doc comment.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub comment: Option<String>,
	/// The column's previous name, when it was renamed. Only meaningful in a
	/// target schema; snapshots never contain it.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub renamed_from: Option<String>,
	/// How the database computes the column from the row's other columns,
	/// when it does: a stored generated column, which writes never set.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub generated: Option<Generated>,
}

impl Column {
	/// A `NOT NULL` column with no default.
	pub fn new(name: impl Into<String>, ty: ColumnType) -> Self {
		Self {
			name: name.into(),
			ty,
			nullable: false,
			default: None,
			comment: None,
			renamed_from: None,
			generated: None,
		}
	}

	/// The same column, computed by the database from `generated`.
	#[must_use]
	pub fn generated(mut self, generated: Generated) -> Self {
		self.generated = Some(generated);
		self
	}

	/// The SQL that computes the column on `dialect`, for a generated column.
	pub fn generated_sql(&self, dialect: Dialect) -> Option<&str> {
		self.generated
			.as_ref()
			.map(|generated| generated.sql(dialect))
	}

	/// The same column, accepting `NULL`.
	#[must_use]
	pub fn nullable(mut self) -> Self {
		self.nullable = true;
		self
	}

	/// The same column, with a default.
	#[must_use]
	pub fn default(mut self, default: DefaultValue) -> Self {
		self.default = Some(default);
		self
	}
}

/// What a column stores, independent of dialect. Each renderer maps these to
/// its own SQL types.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum ColumnType {
	/// `true` or `false`.
	Bool,
	/// A 16-bit signed integer.
	Int16,
	/// A 32-bit signed integer.
	Int32,
	/// A 64-bit signed integer.
	Int64,
	/// A 32-bit float.
	Float32,
	/// A 64-bit float.
	Float64,
	/// An exact decimal. Without a precision, any size.
	Decimal {
		/// Total significant digits.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		precision: Option<u32>,
		/// Digits after the decimal point.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		scale: Option<u32>,
	},
	/// Text, optionally limited to a number of characters.
	Text {
		/// The most characters a value may have.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		max_len: Option<u32>,
	},
	/// Raw bytes.
	Bytes,
	/// A UUID.
	Uuid,
	/// An instant in UTC.
	Timestamp,
	/// A calendar date.
	Date,
	/// A JSON document.
	Json,
	/// An array of another type. Postgres only.
	Array {
		/// The element type.
		element: Box<ColumnType>,
	},
}

impl ColumnType {
	/// Text with no length limit.
	pub const TEXT: Self = Self::Text { max_len: None };

	/// Whether every value of `self` converts to `to` without loss.
	pub(crate) fn widens_to(&self, to: &Self) -> bool {
		if self == to {
			return true;
		}

		match (self, to) {
			(Self::Int16, Self::Int32 | Self::Int64 | Self::Float64)
			| (Self::Int32, Self::Int64 | Self::Float64)
			| (Self::Float32, Self::Float64) => true,
			(Self::Text { max_len: from }, Self::Text { max_len: to }) => {
				match (from, to) {
					(_, None) => true,
					(None, Some(_)) => false,
					(Some(from), Some(to)) => from <= to,
				}
			}
			_ => false,
		}
	}
}

impl fmt::Display for ColumnType {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Bool => f.write_str("bool"),
			Self::Int16 => f.write_str("int16"),
			Self::Int32 => f.write_str("int32"),
			Self::Int64 => f.write_str("int64"),
			Self::Float32 => f.write_str("float32"),
			Self::Float64 => f.write_str("float64"),
			Self::Decimal { precision, scale } => {
				match (precision, scale) {
					(Some(precision), Some(scale)) => write!(f, "decimal({precision}, {scale})"),
					(Some(precision), None) => write!(f, "decimal({precision})"),
					_ => f.write_str("decimal"),
				}
			}
			Self::Text { max_len: Some(max) } => write!(f, "text({max})"),
			Self::Text { max_len: None } => f.write_str("text"),
			Self::Bytes => f.write_str("bytes"),
			Self::Uuid => f.write_str("uuid"),
			Self::Timestamp => f.write_str("timestamp"),
			Self::Date => f.write_str("date"),
			Self::Json => f.write_str("json"),
			Self::Array { element } => write!(f, "{element}[]"),
		}
	}
}

/// The expressions a generated column is computed from, one per dialect,
/// since SQL functions differ between them. Each names the row's other
/// columns, and is stored with the row: Postgres and SQLite both write it
/// as `GENERATED ALWAYS AS (…) STORED`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Generated {
	/// The expression on Postgres.
	pub postgres: String,
	/// The expression on SQLite.
	pub sqlite: String,
}

impl Generated {
	/// The same expression on both dialects.
	pub fn new(sql: impl Into<String>) -> Self {
		let sql = sql.into();
		Self {
			postgres: sql.clone(),
			sqlite: sql,
		}
	}

	/// The expression for `dialect`.
	pub fn sql(&self, dialect: Dialect) -> &str {
		match dialect {
			Dialect::Postgres => &self.postgres,
			Dialect::Sqlite => &self.sqlite,
		}
	}
}

/// A read-only query the database keeps under a name, read like a table.
///
/// A plain view runs its query whenever it's read. A materialized one keeps
/// the rows its query returned when it was last refreshed: a real
/// materialized view on Postgres, and a table the refresh refills on SQLite.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
	/// The view's name, with an optional namespace.
	pub name: TableName,
	/// The columns its query returns, in order.
	pub columns: Vec<Column>,
	/// Its query, one per dialect.
	pub sql: Generated,
	/// Whether it keeps its rows until refreshed.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub materialized: bool,
	/// Documentation, from the struct's doc comment.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub comment: Option<String>,
}

impl View {
	/// A plain view.
	pub fn new(name: TableName, columns: Vec<Column>, sql: Generated) -> Self {
		Self {
			name,
			columns,
			sql,
			materialized: false,
			comment: None,
		}
	}

	/// Whether `other` is the same view on `dialect`: a migration recreates
	/// it otherwise. The other dialect's query doesn't matter.
	pub fn same_on(&self, other: &Self, dialect: Dialect) -> bool {
		self.name == other.name
			&& self.columns == other.columns
			&& self.sql.sql(dialect) == other.sql.sql(dialect)
			&& self.materialized == other.materialized
			&& self.comment == other.comment
	}
}

/// The value the database fills in when an insert omits a column.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DefaultValue {
	/// A boolean.
	Bool(bool),
	/// An integer.
	Integer(i64),
	/// A float.
	Float(f64),
	/// Text.
	Text(String),
	/// The current time when the row is inserted.
	Now,
	/// A SQL expression, written for the schema's dialect.
	Sql(String),
}

impl fmt::Display for DefaultValue {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Bool(value) => write!(f, "{value}"),
			Self::Integer(value) => write!(f, "{value}"),
			Self::Float(value) => write!(f, "{value}"),
			Self::Text(value) => write!(f, "'{value}'"),
			Self::Now => f.write_str("now"),
			Self::Sql(sql) => write!(f, "({sql})"),
		}
	}
}

/// An index. Unique constraints are unique indexes, so that they can be
/// partial (`unique(except = deleted_at)`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
	/// The index's name, unique within the schema.
	pub name: String,
	/// The indexed columns, in order.
	pub columns: Vec<String>,
	/// Whether no two rows may share these columns' values.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub unique: bool,
	/// Only rows matching this are indexed.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub predicate: Option<Predicate>,
}

impl Index {
	/// A plain index with the conventional name.
	pub fn new(table: &TableName, columns: &[&str]) -> Self {
		Self {
			name: crate::constraint_name(table, columns, "idx"),
			columns: columns.iter().map(ToString::to_string).collect(),
			unique: false,
			predicate: None,
		}
	}

	/// A unique index with the conventional name.
	pub fn unique(table: &TableName, columns: &[&str]) -> Self {
		Self {
			name: crate::constraint_name(table, columns, "key"),
			columns: columns.iter().map(ToString::to_string).collect(),
			unique: true,
			predicate: None,
		}
	}

	/// The same index, covering only rows that match `predicate`.
	#[must_use]
	pub fn partial(mut self, predicate: Predicate) -> Self {
		self.predicate = Some(predicate);
		self
	}
}

/// Which rows a partial index covers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Predicate {
	/// Rows where the column is `NULL`, such as rows that aren't soft-deleted.
	IsNull(String),
	/// Rows where the column is not `NULL`.
	IsNotNull(String),
	/// A SQL expression, written for the schema's dialect.
	Sql(String),
}

/// A foreign key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForeignKey {
	/// The constraint's name.
	pub name: String,
	/// The referencing columns, in this table.
	pub columns: Vec<String>,
	/// The referenced table.
	pub references: TableName,
	/// The referenced columns: its primary key or a unique index.
	pub referenced_columns: Vec<String>,
	/// What happens to this row when the referenced row is deleted.
	#[serde(default, skip_serializing_if = "ReferentialAction::is_no_action")]
	pub on_delete: ReferentialAction,
	/// Whether the check waits until the transaction commits.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub deferred: bool,
}

impl ForeignKey {
	/// A foreign key with the conventional name.
	pub fn new(
		table: &TableName,
		columns: &[&str],
		references: TableName,
		referenced_columns: &[&str],
	) -> Self {
		Self {
			name: crate::constraint_name(table, columns, "fkey"),
			columns: columns.iter().map(ToString::to_string).collect(),
			references,
			referenced_columns: referenced_columns.iter().map(ToString::to_string).collect(),
			on_delete: ReferentialAction::NoAction,
			deferred: false,
		}
	}

	/// The same foreign key, with this delete action.
	#[must_use]
	pub fn on_delete(mut self, action: ReferentialAction) -> Self {
		self.on_delete = action;
		self
	}
}

/// What happens to referencing rows when a referenced row is deleted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferentialAction {
	/// Fail the delete at the end of the statement (the SQL default).
	#[default]
	NoAction,
	/// Fail the delete immediately.
	Restrict,
	/// Delete the referencing rows too.
	Cascade,
	/// Set the referencing columns to `NULL`.
	SetNull,
}

impl ReferentialAction {
	#[expect(
		clippy::trivially_copy_pass_by_ref,
		reason = "serde's `skip_serializing_if` passes a reference"
	)]
	fn is_no_action(&self) -> bool {
		matches!(self, Self::NoAction)
	}

	/// The action's SQL keywords.
	pub fn sql(self) -> &'static str {
		match self {
			Self::NoAction => "NO ACTION",
			Self::Restrict => "RESTRICT",
			Self::Cascade => "CASCADE",
			Self::SetNull => "SET NULL",
		}
	}
}

/// A check constraint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
	/// The constraint's name.
	pub name: String,
	/// What every row must satisfy.
	pub rule: CheckRule,
}

/// What a check constraint requires. The structured rules are the ones the
/// Rust side also validates; [`CheckRule::Sql`] is database-only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum CheckRule {
	/// A number between two inclusive bounds.
	Range {
		/// The constrained column.
		column: String,
		/// The smallest allowed value.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		min: Option<f64>,
		/// The largest allowed value.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		max: Option<f64>,
	},
	/// Text whose length in characters is between two inclusive bounds.
	Length {
		/// The constrained column.
		column: String,
		/// The fewest characters allowed.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		min: Option<u32>,
		/// The most characters allowed.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		max: Option<u32>,
	},
	/// Text that is one of a fixed set of values.
	OneOf {
		/// The constrained column.
		column: String,
		/// The allowed values.
		values: Vec<String>,
	},
	/// Text matching a regular expression. Postgres checks it in the
	/// database; SQLite has no built-in regular expressions, so there it is
	/// checked only by the Rust side.
	Regex {
		/// The constrained column.
		column: String,
		/// The pattern.
		pattern: String,
	},
	/// A SQL expression, written for the schema's dialect.
	Sql {
		/// The expression.
		sql: String,
	},
}

impl CheckRule {
	/// The column a structured rule constrains.
	pub fn column(&self) -> Option<&str> {
		match self {
			Self::Range { column, .. }
			| Self::Length { column, .. }
			| Self::OneOf { column, .. }
			| Self::Regex { column, .. } => Some(column),
			Self::Sql { .. } => None,
		}
	}
}
