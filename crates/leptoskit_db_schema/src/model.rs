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
	/// SQLite only: whether the database also holds GeoPackage's metadata
	/// tables, registering each table's geometry column, so GIS tools such
	/// as QGIS open the file as layers. `[db.geo] geopackage = true`.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub geopackage: bool,
}

impl Schema {
	/// A schema with no tables: the baseline before the first migration.
	pub fn empty(dialect: Dialect) -> Self {
		Self {
			format: FORMAT_VERSION,
			dialect,
			tables: Vec::new(),
			views: Vec::new(),
			geopackage: false,
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
			geopackage: false,
		}
	}

	/// The same schema with these views, sorted by name.
	#[must_use]
	pub fn with_views(mut self, mut views: Vec<View>) -> Self {
		views.sort_by(|left, right| left.name.cmp(&right.name));
		self.views = views;
		self
	}

	/// The same schema, keeping GeoPackage's metadata tables or not.
	#[must_use]
	pub fn with_geopackage(mut self, geopackage: bool) -> Self {
		self.geopackage = geopackage;
		self
	}

	/// The GeoPackage layer `table` registers: its geometry column, when the
	/// schema keeps GeoPackage's metadata and the table has one.
	pub fn geopackage_layer(&self, table: &Table) -> Option<GeoPackageLayer> {
		if !self.geopackage {
			return None;
		}

		table.columns.iter().find_map(|column| {
			let (shape, srid) = column.ty.geo()?;
			Some(GeoPackageLayer {
				column: column.name.clone(),
				shape,
				srid,
			})
		})
	}

	/// The Postgres extensions the schema's columns need: `postgis` for
	/// geospatial columns.
	pub fn extensions(&self) -> Vec<&'static str> {
		let columns = || {
			self.tables
				.iter()
				.flat_map(|table| &table.columns)
				.chain(self.views.iter().flat_map(|view| &view.columns))
		};
		let mut extensions = Vec::new();

		if columns().any(|column| column.ty.geo().is_some()) {
			extensions.push(POSTGIS);
		}

		extensions
	}

	/// The view with this name, if there is one.
	pub fn view(&self, name: &TableName) -> Option<&View> {
		self.views.iter().find(|view| &view.name == name)
	}

	/// The views another view of the schema may read, by
	/// [`View::may_read`], each before the views that read it, and in name
	/// order otherwise.
	///
	/// # Errors
	///
	/// The views that read each other in a cycle, which no database can
	/// create.
	pub fn views_in_order(&self) -> Result<Vec<&View>, Vec<TableName>> {
		let depends_on = |view: &View, other: &View| {
			view.name != other.name && view.may_read(&other.name, self.dialect)
		};
		let mut ordered: Vec<&View> = Vec::with_capacity(self.views.len());
		let mut pending: Vec<&View> = self.views.iter().collect();

		while !pending.is_empty() {
			// The first view, by name, whose every dependency is created.
			let ready = pending
				.iter()
				.position(|view| !pending.iter().any(|other| depends_on(view, other)));

			let Some(ready) = ready else {
				return Err(pending.iter().map(|view| view.name.clone()).collect());
			};

			ordered.push(pending.remove(ready));
		}

		Ok(ordered)
	}

	/// The views that read `name`, directly or through other views.
	pub fn views_reading(&self, name: &TableName) -> Vec<&View> {
		let mut reached: Vec<&View> = Vec::new();
		let mut frontier = vec![name.clone()];

		while let Some(next) = frontier.pop() {
			for view in &self.views {
				if view.name != next
					&& view.may_read(&next, self.dialect)
					&& !reached.iter().any(|seen| seen.name == view.name)
					&& &view.name != name
				{
					reached.push(view);
					frontier.push(view.name.clone());
				}
			}
		}

		reached
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

	/// The same schema without rename hints or merge strategies. Snapshots
	/// are written this way: a rename hint describes how to reach this
	/// schema from the previous one, and means nothing once that migration
	/// exists; a merge strategy is behaviour, not a table's shape, so
	/// changing one needs no migration.
	#[must_use]
	pub fn without_rename_hints(&self) -> Self {
		let mut schema = self.clone();

		for table in &mut schema.tables {
			table.renamed_from = None;
			table.on_account_merge = None;

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
	/// The table's full-text search index, when some of its text columns
	/// are searched.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub search: Option<Search>,
	/// Row-level security: the table's access policy, enforced by Postgres
	/// itself as well as by the database layer. Postgres only.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub row_security: Option<RowSecurity>,
	/// Spatial indexes over geospatial columns, in declaration order.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub geo_indexes: Vec<GeoIndex>,
	/// What happens to the table's rows when two accounts merge and the
	/// absorbed account's data comes along: `#[db::table(on_account_merge =
	/// …)]`. Only meaningful in a target schema; snapshots never contain
	/// it.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub on_account_merge: Option<MergeStrategy>,
}

/// How a table's rows move when one account is merged into another, for
/// the rows that reference the absorbed account.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
	/// Point them at the surviving account. A row that would break a unique
	/// constraint stops the whole merge. The default for tables without a
	/// unique constraint on the reference.
	Reassign,
	/// Point them at the surviving account, dropping the ones that would
	/// break a unique constraint, such as a follow the survivor already has.
	ReassignSkipConflicts,
	/// Drop them: the surviving account's rows are the ones that count.
	KeepSurvivor,
	/// For a table with one row per account: keep whichever row was
	/// updated more recently, by its `updated_at` column.
	KeepNewest,
	/// The app moves them itself, in the merge's transaction.
	Custom,
}

impl MergeStrategy {
	/// The strategy's name, as `on_account_merge` spells it.
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Reassign => "reassign",
			Self::ReassignSkipConflicts => "reassign_skip_conflicts",
			Self::KeepSurvivor => "keep_survivor",
			Self::KeepNewest => "keep_newest",
			Self::Custom => "custom",
		}
	}
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
			search: None,
			row_security: None,
			geo_indexes: Vec::new(),
			on_account_merge: None,
		}
	}

	/// The same table, with this full-text search index.
	#[must_use]
	pub fn with_search(mut self, search: Search) -> Self {
		self.search = Some(search);
		self
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
	/// A planar shape in a coordinate system: PostGIS's `geometry` on
	/// Postgres, GeoPackage's binary encoding in a `BLOB` on SQLite.
	Geometry {
		/// The kind of shape every value has.
		shape: GeoShape,
		/// The coordinate system's EPSG code, such as 27700.
		srid: u32,
	},
	/// A shape in longitude and latitude on the WGS 84 spheroid, measured in
	/// metres: PostGIS's `geography` on Postgres, a `BLOB` on SQLite.
	Geography {
		/// The kind of shape every value has.
		shape: GeoShape,
	},
}

impl ColumnType {
	/// Text with no length limit.
	pub const TEXT: Self = Self::Text { max_len: None };

	/// The shape and SRID of a geospatial column: geography is always in
	/// WGS 84, 4326.
	pub fn geo(&self) -> Option<(GeoShape, u32)> {
		match self {
			Self::Geometry { shape, srid } => Some((*shape, *srid)),
			Self::Geography { shape } => Some((*shape, WGS84)),
			_ => None,
		}
	}

	/// Whether every value of `self` converts to `to` without loss.
	pub(crate) fn widens_to(&self, to: &Self) -> bool {
		if self == to {
			return true;
		}

		match (self, to) {
			// Any shape fits a column of every shape, in the same system.
			(Self::Int16, Self::Int32 | Self::Int64 | Self::Float64)
			| (Self::Int32, Self::Int64 | Self::Float64)
			| (Self::Float32, Self::Float64)
			| (
				Self::Geography { .. },
				Self::Geography {
					shape: GeoShape::Geometry,
				},
			) => true,
			(
				Self::Geometry { srid: from, .. },
				Self::Geometry {
					shape: GeoShape::Geometry,
					srid: to,
				},
			) => from == to,
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
			Self::Geometry { shape, srid } => write!(f, "geometry({shape}, {srid})"),
			Self::Geography { shape } => write!(f, "geography({shape})"),
		}
	}
}

/// The EPSG code of WGS 84 longitude and latitude, which every geography
/// column uses.
pub const WGS84: u32 = 4326;

/// The largest SRID PostGIS accepts.
pub const MAX_SRID: u32 = 999_999;

/// The Postgres extension geospatial columns need.
pub const POSTGIS: &str = "postgis";

/// The kind of shape a geospatial column holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeoShape {
	/// One position.
	Point,
	/// A line through two or more positions.
	LineString,
	/// An area: an outer ring and any holes.
	Polygon,
	/// Several points.
	MultiPoint,
	/// Several lines.
	MultiLineString,
	/// Several areas.
	MultiPolygon,
	/// Shapes of any kinds together.
	GeometryCollection,
	/// Any one shape.
	Geometry,
}

impl GeoShape {
	/// Every shape.
	pub const ALL: [Self; 8] = [
		Self::Point,
		Self::LineString,
		Self::Polygon,
		Self::MultiPoint,
		Self::MultiLineString,
		Self::MultiPolygon,
		Self::GeometryCollection,
		Self::Geometry,
	];

	/// The name PostGIS types and GeoJSON use: `Point`, `MultiPolygon`.
	pub fn name(self) -> &'static str {
		match self {
			Self::Point => "Point",
			Self::LineString => "LineString",
			Self::Polygon => "Polygon",
			Self::MultiPoint => "MultiPoint",
			Self::MultiLineString => "MultiLineString",
			Self::MultiPolygon => "MultiPolygon",
			Self::GeometryCollection => "GeometryCollection",
			Self::Geometry => "Geometry",
		}
	}

	/// The shape with this [`GeoShape::name`].
	pub fn from_name(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|shape| shape.name() == name)
	}
}

impl fmt::Display for GeoShape {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.name())
	}
}

/// A spatial index over one geospatial column: GiST on Postgres; on SQLite,
/// an R*Tree of each row's bounding box with a table numbering the rows by
/// key, both kept current by triggers, so raw SQL writes are indexed too.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeoIndex {
	/// The indexed column.
	pub column: String,
}

impl GeoIndex {
	/// An index over `column`.
	pub fn new(column: impl Into<String>) -> Self {
		Self {
			column: column.into(),
		}
	}

	/// The GiST index on Postgres.
	pub fn index_name(&self, table: &TableName) -> String {
		crate::constraint_name(table, &[&self.column], "gist")
	}

	/// The R*Tree holding each row's bounding box on SQLite:
	/// `rtree_<table>_<column>`, as GeoPackage names it.
	pub fn rtree_table(&self, table: &TableName) -> String {
		crate::shorten_identifier(&format!("rtree_{}_{}", table.flat(), self.column))
	}

	/// The table numbering the indexed rows by key on SQLite.
	pub fn rows_table(&self, table: &TableName) -> String {
		crate::shorten_identifier(&format!("rtree_{}_{}_rows", table.flat(), self.column))
	}

	/// The trigger that indexes a row after `event` (`insert`, `update`, or
	/// `delete`) on SQLite.
	pub fn trigger_name(&self, table: &TableName, event: &str) -> String {
		crate::shorten_identifier(&format!("rtree_{}_{}_{event}", table.flat(), self.column))
	}
}

/// The geometry column a table registers as a GeoPackage layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeoPackageLayer {
	/// The column.
	pub column: String,
	/// Its shape.
	pub shape: GeoShape,
	/// Its coordinate system.
	pub srid: u32,
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
	/// Indexes over a materialized view's columns. A unique one, without a
	/// predicate, lets Postgres refresh it while it's being read.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub indexes: Vec<Index>,
	/// Whether Postgres reads the view's tables as whoever reads the view,
	/// rather than as its owner, so their row-level security filters it for
	/// each request: Postgres's `security_invoker`. Plain Postgres views
	/// only.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub security_invoker: bool,
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
			indexes: Vec::new(),
			security_invoker: false,
		}
	}

	/// Whether the view's query on `dialect` may read `other`, a table or
	/// view: whether it names it. The query isn't parsed, so a column that
	/// shares `other`'s name counts too, which only ever orders views more
	/// carefully than they need.
	pub fn may_read(&self, other: &TableName, dialect: Dialect) -> bool {
		let names = sql_names(self.sql.sql(dialect));
		let flat = other.flat();

		names.iter().any(|name| {
			name == &other.name
				|| name == &flat
				|| other
					.namespace
					.as_ref()
					.is_some_and(|namespace| name == &format!("{namespace}.{}", other.name))
		})
	}

	/// Whether `other` is the same view on `dialect`: a migration recreates
	/// it otherwise. The other dialect's query doesn't matter.
	pub fn same_on(&self, other: &Self, dialect: Dialect) -> bool {
		self.name == other.name
			&& self.columns == other.columns
			&& self.sql.sql(dialect) == other.sql.sql(dialect)
			&& self.materialized == other.materialized
			&& self.comment == other.comment
			&& self.indexes == other.indexes
			&& self.security_invoker == other.security_invoker
	}
}

/// The names a query mentions: each identifier, bare or quoted, and each
/// `a.b` pair, skipping string literals and comments.
fn sql_names(sql: &str) -> Vec<String> {
	let mut names: Vec<String> = Vec::new();
	let mut chars = sql.chars().peekable();
	// Whether the last name was followed by a `.`, so the next one joins it.
	let mut dotted = false;

	while let Some(next) = chars.next() {
		let name = match next {
			'\'' => {
				while let Some(inner) = chars.next() {
					if inner == '\'' && chars.next_if_eq(&'\'').is_none() {
						break;
					}
				}
				dotted = false;
				continue;
			}
			'-' if chars.peek() == Some(&'-') => {
				for inner in chars.by_ref() {
					if inner == '\n' {
						break;
					}
				}
				dotted = false;
				continue;
			}
			'"' => {
				let mut name = String::new();
				while let Some(inner) = chars.next() {
					if inner == '"' && chars.next_if_eq(&'"').is_none() {
						break;
					}
					name.push(inner);
				}
				name
			}
			'.' => {
				dotted = !names.is_empty();
				continue;
			}
			character if character.is_alphanumeric() || character == '_' => {
				let mut name = String::from(character);
				while let Some(inner) =
					chars.next_if(|inner| inner.is_alphanumeric() || *inner == '_')
				{
					name.push(inner);
				}
				name.to_lowercase()
			}
			_ => {
				if !next.is_whitespace() {
					dotted = false;
				}
				continue;
			}
		};

		if std::mem::take(&mut dotted)
			&& let Some(previous) = names.last()
		{
			let joined = format!("{previous}.{name}");
			names.push(joined);
		}

		names.push(name);
	}

	names
}

/// A table's full-text search index: the text columns it searches, how
/// much a match in each counts, and the language their words are in.
///
/// Postgres keeps a stored generated `tsvector` column, [`SEARCH_COLUMN`],
/// with a GIN index. SQLite keeps an FTS5 table holding a copy of the
/// searched text, and a table mapping each of its rows to the searched
/// row's key, both kept up to date by triggers, so raw SQL writes are
/// indexed too.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Search {
	/// How words are split and folded: `simple` (lowercased, every word
	/// kept) on both dialects, `english` (stemmed: Postgres's `english`
	/// configuration, SQLite's `porter` tokenizer), or, on Postgres, any
	/// other text search configuration it has.
	pub language: String,
	/// The searched columns, in declaration order.
	pub columns: Vec<SearchColumn>,
}

impl Search {
	/// Stemmed English on both dialects.
	pub const ENGLISH: &'static str = "english";
	/// The language every dialect supports.
	pub const SIMPLE: &'static str = "simple";

	/// An index over `columns`, in `language`.
	pub fn new(language: impl Into<String>, columns: Vec<SearchColumn>) -> Self {
		Self {
			language: language.into(),
			columns,
		}
	}

	/// Whether the index reads `column`.
	pub fn reads(&self, column: &str) -> bool {
		self.columns.iter().any(|searched| searched.name == column)
	}

	/// The GIN index over [`SEARCH_COLUMN`] on Postgres.
	pub fn index_name(table: &TableName) -> String {
		crate::constraint_name(table, &[SEARCH_COLUMN], "idx")
	}

	/// The FTS5 table holding a copy of the searched text on SQLite.
	pub fn text_table(table: &TableName) -> String {
		crate::shorten_identifier(&format!("{}_{SEARCH_COLUMN}", table.flat()))
	}

	/// The table mapping each FTS5 row to the searched row's key on SQLite.
	pub fn rows_table(table: &TableName) -> String {
		crate::shorten_identifier(&format!("{}_{SEARCH_COLUMN}_rows", table.flat()))
	}

	/// The trigger that indexes a row after `event` (`insert`, `update`, or
	/// `delete`) on SQLite.
	pub fn trigger_name(table: &TableName, event: &str) -> String {
		crate::shorten_identifier(&format!("{}_{SEARCH_COLUMN}_{event}", table.flat()))
	}
}

/// The name of the generated `tsvector` column that holds a searched
/// table's words on Postgres. Queries never select it.
pub const SEARCH_COLUMN: &str = "lk_search";

/// A table's access policy as Postgres row-level security: one policy per
/// command the table's rules govern, each a SQL condition over the row and
/// the request's settings (`current_setting('leptoskit.…')`). A command
/// without a policy is denied, as the database layer denies it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowSecurity {
	/// The policies, at most one per command.
	pub policies: Vec<RowPolicy>,
}

/// The setting that lets a connection bypass every policy, as the database
/// layer's `Db::system` and the migrator do.
pub const BYPASS_SETTING: &str = "leptoskit.bypass";

/// The prefix of every setting a policy reads: `leptoskit.viewer`.
pub const SETTING_PREFIX: &str = "leptoskit.";

/// One row-level security policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowPolicy {
	/// The command it governs.
	pub command: PolicyCommand,
	/// Which rows the command may see or change, in Postgres SQL. Every
	/// command but `insert` has one.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub using: Option<String>,
	/// What rows the command may leave behind, in Postgres SQL. `insert` and
	/// `update` have one.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub check: Option<String>,
}

impl RowPolicy {
	/// The policy's name on `table`: the table's flat name and the command.
	pub fn name(&self, table: &TableName) -> String {
		crate::constraint_name(table, &[], self.command.keyword_lower())
	}
}

/// A command a row-level security policy governs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyCommand {
	/// Reading rows.
	Select,
	/// Inserting rows.
	Insert,
	/// Changing rows.
	Update,
	/// Deleting rows.
	Delete,
}

impl PolicyCommand {
	/// The command's SQL keyword.
	pub fn keyword(self) -> &'static str {
		match self {
			Self::Select => "SELECT",
			Self::Insert => "INSERT",
			Self::Update => "UPDATE",
			Self::Delete => "DELETE",
		}
	}

	fn keyword_lower(self) -> &'static str {
		match self {
			Self::Select => "select",
			Self::Insert => "insert",
			Self::Update => "update",
			Self::Delete => "delete",
		}
	}
}

/// The integer column that numbers a searched table's rows in its SQLite
/// companion tables.
pub const SEARCH_ROW: &str = "lk_row";

/// One searched column.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchColumn {
	/// The column's name.
	pub name: String,
	/// How much a match in it counts towards a row's rank.
	pub weight: SearchWeight,
}

impl SearchColumn {
	/// A searched column with this weight.
	pub fn new(name: impl Into<String>, weight: SearchWeight) -> Self {
		Self {
			name: name.into(),
			weight,
		}
	}
}

/// How much a match in a searched column counts: `A` most, then `B`, `C`,
/// and `D`, as in Postgres's `setweight`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SearchWeight {
	/// The most: titles and names.
	A,
	/// Summaries and headings.
	B,
	/// Body text.
	C,
	/// The least, and Postgres's default.
	D,
}

impl SearchWeight {
	/// The letter Postgres's `setweight` takes.
	pub fn letter(self) -> char {
		match self {
			Self::A => 'A',
			Self::B => 'B',
			Self::C => 'C',
			Self::D => 'D',
		}
	}

	/// The multiplier a match counts with: Postgres's default `ts_rank`
	/// weights, which SQLite's `bm25` is given too, so ranks order alike.
	pub fn factor(self) -> f64 {
		match self {
			Self::A => 1.0,
			Self::B => 0.4,
			Self::C => 0.2,
			Self::D => 0.1,
		}
	}
}

impl fmt::Display for SearchWeight {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.letter())
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
	/// A shape that is valid as OGC defines it: rings closed and not
	/// crossing themselves, holes inside their polygon. PostGIS's
	/// `ST_IsValid` on Postgres, the `geo` crate's validation on SQLite and in
	/// Rust.
	ValidGeometry {
		/// The constrained column.
		column: String,
	},
}

impl CheckRule {
	/// The column a structured rule constrains.
	pub fn column(&self) -> Option<&str> {
		match self {
			Self::Range { column, .. }
			| Self::Length { column, .. }
			| Self::OneOf { column, .. }
			| Self::Regex { column, .. }
			| Self::ValidGeometry { column } => Some(column),
			Self::Sql { .. } => None,
		}
	}
}
