//! Tables, and the schema they add up to.

use leptoskit_db_schema::Check;
use leptoskit_db_schema::CheckRule;
use leptoskit_db_schema::DefaultValue;
use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::ForeignKey;
use leptoskit_db_schema::Index;
use leptoskit_db_schema::Predicate;
use leptoskit_db_schema::ReferentialAction;
use leptoskit_db_schema::Schema;
use leptoskit_db_schema::SchemaError;
use leptoskit_db_schema::TableName;
use leptoskit_db_schema::constraint_name;

use crate::Column;
use crate::DecodeError;
use crate::EncodeError;
use crate::Value;

/// A table declared with `#[db::table]`. The attribute implements it; you
/// don't.
pub trait Table: Sized + 'static {
	/// The table's name, without its namespace.
	const NAME: &'static str;
	/// The table's namespace, such as `auth`.
	const NAMESPACE: Option<&'static str>;
	/// The column names, in declaration order.
	const COLUMNS: &'static [&'static str];
	/// The primary key's columns.
	const KEY: &'static [&'static str];
	/// The column the ORM sets when a row is inserted.
	const CREATED_AT: Option<&'static str>;
	/// The column the ORM sets on every write.
	const UPDATED_AT: Option<&'static str>;
	/// The soft-delete column: deleted rows are hidden unless asked for.
	const DELETED_AT: Option<&'static str>;

	/// The version column: `save` checks it is unchanged and increments it,
	/// so a write based on a stale read fails instead of overwriting.
	const VERSION: Option<&'static str> = None;

	/// The schema pack that owns the table, if one does. Pack tables must be
	/// portable, and only the pack's crate writes to them.
	const PACK: Option<&'static str> = None;

	/// The columns the database computes, `#[db(generated = "…")]`, which
	/// writes never set.
	const GENERATED: &'static [&'static str] = &[];

	/// The columns recording who inserted the row and who last wrote it,
	/// `#[db(created_by)]` and `#[db(updated_by)]`, which writes fill in from
	/// the [`Actor`](crate::Actor).
	const CREATED_BY: Option<&'static str> = None;

	/// See [`Self::CREATED_BY`].
	const UPDATED_BY: Option<&'static str> = None;

	/// The `#[db(slug(from = …))]` columns, each with the column its slug is
	/// made from. Inserts fill them in, unique within the table.
	const SLUGS: &'static [(&'static str, &'static str)] = &[];

	/// The `secret` columns, which the audit log never records.
	const SECRET: &'static [&'static str] = &[];

	/// Whether every change is recorded in the audit log:
	/// `#[db::table(audit)]`.
	const AUDIT: bool = false;

	/// Whether every write is captured in the change log, by triggers that
	/// migrations create, so live subscribers hear of it:
	/// `#[db::table(live)]`.
	const LIVE: bool = false;

	/// The columns a live table's capture triggers copy into the change
	/// log: every column except `secret` ones and those marked
	/// `#[db(live = skip)]`. Empty when the table isn't live.
	const LIVE_COLUMNS: &'static [&'static str] = &[];

	/// The hooks the table declares with `hooks(...)`.
	const HOOKS: crate::Hooks = crate::Hooks::NONE;

	/// Whether the table is a view, which nothing writes to.
	const READ_ONLY: bool = false;

	/// The tables a read of this one reads in its place: those a view's
	/// query reads, `#[db::view(reads(…))]`. `None` for a view that doesn't
	/// say, which a [`Tracked`](crate::Tracked) connection refuses to read.
	#[doc(hidden)]
	fn reads_through() -> Option<Vec<TableName>> {
		Some(Vec::new())
	}

	/// The supertype of a variant of a polymorphic table,
	/// `#[db::table(extends = …)]`, whose policies the table inherits.
	#[doc(hidden)]
	const SUPERTYPE: Option<crate::__private::polymorphic::Supertype> = None;

	/// The geospatial columns with a spatial index, `#[db(geo(index))]`.
	const GEO_INDEXED: &'static [&'static str] = &[];

	/// The typed columns, for filters and orderings.
	type Columns: 'static;

	/// The typed columns, generated as `{Table}Columns`.
	fn columns() -> &'static Self::Columns;

	/// Checks every rule the columns declare. Writes call it first.
	fn validate(&self) -> Result<(), crate::ValidationErrors>;

	/// The table's full definition, as migrations see it.
	fn definition() -> leptoskit_db_schema::Table;

	/// Every column's value, in [`Self::COLUMNS`] order.
	fn to_values(&self) -> Result<Vec<Value>, EncodeError>;

	/// A row from values in [`Self::COLUMNS`] order.
	fn from_values(values: Vec<Value>) -> Result<Self, DecodeError>;

	/// Sets the `updated_at` column, if there is one, to now.
	fn touch(&mut self);

	/// Sets the soft-delete column, if there is one, to now.
	fn mark_deleted(&mut self);

	/// Increments the version column, if there is one, after a save.
	fn bump_version(&mut self) {}

	/// Fills in the `created_by` column, when `inserting`, and the
	/// `updated_by` column with `actor`. Fails with
	/// [`DbError::NoActor`](crate::DbError::NoActor) when a column needs
	/// someone acting and nobody is.
	fn stamp(&mut self, actor: Option<crate::Actor>, inserting: bool) -> crate::Result<()> {
		let _ = (actor, inserting);
		Ok(())
	}

	/// Runs the table's [`BeforeWrite`](crate::BeforeWrite) hook, if it has
	/// one.
	fn hook_before_write(&mut self, cx: &crate::WriteContext<'_, Self>) -> crate::Result<()> {
		let _ = cx;
		Ok(())
	}

	/// Runs the table's [`AfterInsert`](crate::AfterInsert) hook, if it has
	/// one.
	fn hook_after_insert(
		&self,
		connection: &impl crate::Connection,
	) -> impl Future<Output = crate::Result<()>> + Send {
		let _ = connection;
		async { Ok(()) }
	}

	/// Runs the table's [`AfterUpdate`](crate::AfterUpdate) hook, if it has
	/// one.
	fn hook_after_update(
		old: &Self,
		new: &Self,
		connection: &impl crate::Connection,
	) -> impl Future<Output = crate::Result<()>> + Send {
		let _ = (old, new, connection);
		async { Ok(()) }
	}

	/// Runs the table's [`AfterDelete`](crate::AfterDelete) hook, if it has
	/// one.
	fn hook_after_delete(
		&self,
		connection: &impl crate::Connection,
	) -> impl Future<Output = crate::Result<()>> + Send {
		let _ = connection;
		async { Ok(()) }
	}

	/// Inserts a variant row's supertype row, before the row itself.
	#[doc(hidden)]
	fn write_supertype(
		&self,
		connection: &impl crate::Connection,
	) -> impl Future<Output = crate::Result<()>> + Send {
		let _ = connection;
		async { Ok(()) }
	}

	/// Removes the supertype rows of variant rows just removed, by key.
	#[doc(hidden)]
	fn remove_supertypes(
		keys: Vec<Value>,
		connection: &impl crate::Connection,
	) -> impl Future<Output = crate::Result<()>> + Send {
		let _ = (keys, connection);
		async { Ok(()) }
	}

	/// The table's access policy, which `#[db::policy]` declares. Empty means
	/// open to server code.
	fn policies() -> &'static [crate::Policy<Self>] {
		&[]
	}

	/// The table's name with its namespace.
	fn table_name() -> TableName {
		TableName {
			namespace: Self::NAMESPACE.map(ToString::to_string),
			name: Self::NAME.to_string(),
		}
	}

	/// The name with its namespace joined by `_`.
	fn flat_name() -> String {
		Self::table_name().flat()
	}
}

/// One table, registered by `#[db::table]` so that the schema can be exported
/// without listing tables by hand.
#[doc(hidden)]
pub struct TableEntry {
	pub(crate) name: fn() -> TableName,
	pub(crate) definition: fn() -> leptoskit_db_schema::Table,
	pack: Option<&'static str>,
	audit: bool,
	live: bool,
	/// The columns its capture triggers copy, when it's live.
	live_columns: &'static [&'static str],
	/// How to record the table's rows, for the changes the database makes
	/// to them on its own.
	pub(crate) subject: crate::audit::Subject,
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	images: VisibleImages,
	/// The table's policies as row-level security.
	row_security: fn() -> Result<Option<leptoskit_db_schema::RowSecurity>, String>,
	/// What the studio does to the table.
	#[cfg(all(
		feature = "studio",
		debug_assertions,
		any(feature = "postgres", feature = "sqlite")
	))]
	pub(crate) studio: crate::studio::TableOps,
}

/// Builds the statements that check rows of a table, which aren't stored,
/// against a request's `select` policy: [`crate::live::visible_images`].
#[cfg(any(feature = "postgres", feature = "sqlite"))]
type VisibleImages = fn(
	&crate::policy::Access,
	Dialect,
	&[&[Value]],
) -> crate::Result<Option<Vec<crate::Statement>>>;

impl TableEntry {
	pub const fn of<T: Table>() -> Self {
		Self {
			name: T::table_name,
			definition: T::definition,
			pack: T::PACK,
			audit: T::AUDIT,
			live: T::LIVE,
			live_columns: T::LIVE_COLUMNS,
			subject: crate::audit::Subject::of::<T>(),
			#[cfg(any(feature = "postgres", feature = "sqlite"))]
			images: crate::live::visible_images::<T>,
			row_security: crate::policy::row_security::<T>,
			#[cfg(all(
				feature = "studio",
				debug_assertions,
				any(feature = "postgres", feature = "sqlite")
			))]
			studio: crate::studio::TableOps::of::<T>(),
		}
	}

	/// Statements returning the number of each image the access may read,
	/// or `None` when it may read every one.
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	pub(crate) fn visible_images(
		&self,
		access: &crate::policy::Access,
		dialect: Dialect,
		images: &[&[Value]],
	) -> crate::Result<Option<Vec<crate::Statement>>> {
		(self.images)(access, dialect, images)
	}
}

inventory::collect!(TableEntry);

/// Every registered table, by name.
pub(crate) fn entries() -> &'static std::collections::HashMap<TableName, &'static TableEntry> {
	static ENTRIES: std::sync::OnceLock<std::collections::HashMap<TableName, &'static TableEntry>> =
		std::sync::OnceLock::new();
	ENTRIES.get_or_init(|| {
		inventory::iter::<TableEntry>()
			.map(|entry| ((entry.name)(), entry))
			.collect()
	})
}

/// The registered `#[db::table(live)]` table with this name.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) fn live_entry(table: &TableName) -> Option<&'static TableEntry> {
	entries().get(table).copied().filter(|entry| entry.live)
}

/// Every table and view linked into this program, as a schema for
/// `dialect`, with the audit log's `_leptoskit_audit` when any table is
/// audited, and the `live` pack's change log and capture triggers when any
/// is live.
///
/// `leptoskit db` builds a small program that calls this and diffs the result
/// against the latest migration's snapshot. A schema pack's tables must also
/// work on the other dialect, since packs serve apps on either.
pub fn schema(dialect: Dialect) -> Result<Schema, SchemaError> {
	let entries: Vec<&TableEntry> = inventory::iter::<TableEntry>().collect();
	let mut tables: Vec<leptoskit_db_schema::Table> =
		entries.iter().map(|entry| (entry.definition)()).collect();

	if entries.iter().any(|entry| entry.audit) {
		tables.push(crate::AuditEntry::definition());
	}

	// A live table brings the `live` pack: its tables, and on Postgres the
	// functions and sequences that version commits, then each live table's
	// capture triggers.
	let mut triggers = Vec::new();
	let (sequences, functions) = if entries.iter().any(|entry| entry.live) {
		let (sequences, functions, pack) = leptoskit_db_schema::live::pack(dialect);
		triggers.extend(pack);

		for entry in entries.iter().filter(|entry| entry.live) {
			triggers.extend(leptoskit_db_schema::live::capture(
				dialect,
				&(entry.definition)(),
				entry.live_columns,
			));
		}

		tables.extend(crate::live::tables(dialect));
		(sequences, functions)
	} else {
		(Vec::new(), Vec::new())
	};

	let schema = Schema::new(dialect, tables)
		.with_views(crate::view::views())
		.with_routines(sequences, functions, triggers);
	let mut problems = schema.problems();

	let mut packs: Vec<&str> = entries.iter().filter_map(|entry| entry.pack).collect();
	packs.sort_unstable();
	packs.dedup();

	for pack in packs {
		let own: Vec<String> = entries
			.iter()
			.filter(|entry| entry.pack == Some(pack))
			.map(|entry| (entry.definition)().name.to_string())
			.collect();
		// A pack may link to the packs it builds on, such as `idp` to `auth`,
		// but never to an app's tables, so it's checked alongside the other
		// packs. Their own problems are reported under their own names.
		let mut tables: Vec<leptoskit_db_schema::Table> = entries
			.iter()
			.filter(|entry| entry.pack == Some(pack))
			.map(|entry| (entry.definition)())
			.collect();
		tables.extend(
			entries
				.iter()
				.filter(|entry| entry.pack.is_some_and(|other| other != pack))
				.map(|entry| (entry.definition)()),
		);

		if let Err(SchemaError::Invalid(found)) = Schema::validate_portable(&tables) {
			problems.extend(
				found
					.into_iter()
					.filter(|problem| {
						own.iter().any(|table| {
							problem
								.location
								.strip_prefix(table.as_str())
								.is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
						})
					})
					.map(|mut problem| {
						problem.message = format!("pack `{pack}`: {}", problem.message);
						problem
					}),
			);
		}
	}

	if problems.is_empty() {
		Ok(schema)
	} else {
		Err(SchemaError::Invalid(problems))
	}
}

/// [`schema`], with every table's access policy also enforced by Postgres
/// as row-level security, for `[db] policies = "rls"`. Every rule must read
/// the request through settings (`cx.setting::<G>()`) rather than as Rust
/// values (`cx.global`), so the database can evaluate it.
///
/// Plain views read their tables as their reader (`security_invoker`), so
/// the tables' policies filter them too, whoever owns them. The audit log
/// gets the policy [`AuditReader`](crate::AuditReader) can express to
/// Postgres: a request holding any reader reads it.
///
/// # Errors
///
/// The schema's own problems, and each table whose rules can't be enforced
/// by the database, or settings registered twice under one name or with an
/// invalid one.
pub fn schema_with_row_security(dialect: Dialect) -> Result<Schema, SchemaError> {
	let mut schema = schema(dialect)?;
	let mut problems = Vec::new();

	for entry in inventory::iter::<TableEntry>() {
		let name = (entry.name)();

		match (entry.row_security)() {
			Ok(security) => {
				if let Some(table) = schema.tables.iter_mut().find(|table| table.name == name) {
					table.row_security = security;
				}
			}
			Err(message) => {
				problems.push(leptoskit_db_schema::Problem {
					location: name.to_string(),
					message: format!("row-level security: {message}"),
				});
			}
		}
	}

	let audit = crate::AuditEntry::table_name();

	if let Some(table) = schema.tables.iter_mut().find(|table| table.name == audit) {
		table.row_security = Some(crate::audit::row_security());
	}

	if dialect == Dialect::Postgres {
		for view in schema.views.iter_mut().filter(|view| !view.materialized) {
			view.security_invoker = true;
		}
	}

	let mut names: Vec<(&str, &str)> = Vec::new();

	for entry in crate::settings::entries() {
		let type_name = (entry.type_name)();
		let snake = entry
			.name
			.chars()
			.next()
			.is_some_and(|first| first.is_ascii_lowercase())
			&& entry.name.chars().all(|character| {
				character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
			});

		if !snake {
			problems.push(leptoskit_db_schema::Problem {
				location: format!("setting `{type_name}`"),
				message: format!(
					"its name, `{}`, must be lowercase snake case, starting with a letter",
					entry.name
				),
			});
		}

		match names.iter().find(|(name, _)| *name == entry.name) {
			Some((_, other)) if *other != type_name => {
				problems.push(leptoskit_db_schema::Problem {
					location: format!("setting `{type_name}`"),
					message: format!("`{other}` is named `{}` too", entry.name),
				});
			}
			Some(_) => {}
			None => names.push((entry.name, type_name)),
		}
	}

	schema
		.problems()
		.into_iter()
		.for_each(|problem| problems.push(problem));

	if problems.is_empty() {
		Ok(schema)
	} else {
		Err(SchemaError::Invalid(problems))
	}
}

/// The environment variable saying whether `export_schema!` exports the
/// policies as row-level security: `rls`, or `orm` (the default).
pub const POLICIES_ENV: &str = "LEPTOSKIT_DB_POLICIES";

/// The environment variable saying whether `export_schema!` keeps
/// GeoPackage's metadata tables on SQLite: `1`, or `0` (the default), from
/// `[db.geo] geopackage`.
pub const GEOPACKAGE_ENV: &str = "LEPTOSKIT_DB_GEOPACKAGE";

/// Decodes the next value of a row, locating any error at its column.
#[doc(hidden)]
pub fn decode_next<T: Column>(
	values: &mut impl Iterator<Item = Value>,
	table: &str,
	column: &str,
) -> Result<T, DecodeError> {
	let value = values
		.next()
		.ok_or_else(|| DecodeError::custom("the row has too few columns").at(table, column))?;
	T::from_value(value).map_err(|error| error.at(table, column))
}

/// Builds a table's definition. `#[db::table]` generates one call per column
/// and rule, so the generated code reads like the declaration.
#[doc(hidden)]
pub struct TableBuilder {
	table: leptoskit_db_schema::Table,
}

impl TableBuilder {
	pub fn new(
		namespace: Option<&str>,
		name: &str,
		comment: Option<&str>,
		renamed_from: Option<&str>,
	) -> Self {
		let name = TableName {
			namespace: namespace.map(ToString::to_string),
			name: name.to_string(),
		};
		let mut table = leptoskit_db_schema::Table::new(name, Vec::new(), Vec::new());
		table.comment = comment.map(ToString::to_string);
		table.renamed_from = renamed_from.map(|from| {
			TableName {
				namespace: namespace.map(ToString::to_string),
				name: from.to_string(),
			}
		});
		Self { table }
	}

	pub fn column<T: Column>(
		&mut self,
		name: &str,
		default: Option<DefaultValue>,
		comment: Option<&str>,
		renamed_from: Option<&str>,
	) {
		self.table.columns.push(leptoskit_db_schema::Column {
			name: name.to_string(),
			ty: T::column_type(),
			nullable: T::nullable(),
			default,
			comment: comment.map(ToString::to_string),
			renamed_from: renamed_from.map(ToString::to_string),
			generated: None,
		});

		if let Some(values) = T::allowed_values() {
			self.check(
				name,
				"values",
				CheckRule::OneOf {
					column: name.to_string(),
					values: values.iter().map(ToString::to_string).collect(),
				},
			);
		}

		let rules = T::rules();

		if rules.min_len.is_some() || rules.max_len.is_some() {
			self.length(name, rules.min_len, rules.max_len);
		}

		if rules.min.is_some() || rules.max.is_some() {
			self.range(name, rules.min, rules.max);
		}

		if let Some(pattern) = rules.pattern {
			self.pattern(name, pattern);
		}
	}

	/// Makes the column just added one the database computes, with an
	/// expression for each dialect.
	pub fn generated(&mut self, column: &str, postgres: &str, sqlite: &str) {
		if let Some(column) = self
			.table
			.columns
			.iter_mut()
			.find(|candidate| candidate.name == column)
		{
			column.generated = Some(leptoskit_db_schema::Generated {
				postgres: postgres.to_string(),
				sqlite: sqlite.to_string(),
			});
		}
	}

	pub fn key(&mut self, columns: &[&str]) {
		self.table.primary_key = columns.iter().map(ToString::to_string).collect();
	}

	pub fn unique(&mut self, columns: &[&str], except: Option<&str>) {
		let mut index = Index::unique(&self.table.name, columns);

		if let Some(except) = except {
			index = index.partial(Predicate::IsNull(except.to_string()));
		}

		self.table.indexes.push(index);
	}

	pub fn index(&mut self, columns: &[&str]) {
		self.table
			.indexes
			.push(Index::new(&self.table.name, columns));
	}

	/// A text length limit. A maximum alone narrows the column type, so that
	/// Postgres stores it as `varchar(n)`.
	pub fn length(&mut self, column: &str, min: Option<u32>, max: Option<u32>) {
		let narrowed = self
			.table
			.columns
			.iter_mut()
			.find(|candidate| candidate.name == column)
			.is_some_and(|column| {
				match &mut column.ty {
					leptoskit_db_schema::ColumnType::Text { max_len } if min.is_none() => {
						*max_len = max;
						true
					}
					_ => false,
				}
			});

		if !narrowed {
			self.check(
				column,
				"length",
				CheckRule::Length {
					column: column.to_string(),
					min,
					max,
				},
			);
		}
	}

	pub fn range(&mut self, column: &str, min: Option<f64>, max: Option<f64>) {
		self.check(
			column,
			"range",
			CheckRule::Range {
				column: column.to_string(),
				min,
				max,
			},
		);
	}

	pub fn one_of(&mut self, column: &str, values: &[&str]) {
		self.check(
			column,
			"values",
			CheckRule::OneOf {
				column: column.to_string(),
				values: values.iter().map(ToString::to_string).collect(),
			},
		);
	}

	pub fn pattern(&mut self, column: &str, pattern: &str) {
		self.check(
			column,
			"pattern",
			CheckRule::Regex {
				column: column.to_string(),
				pattern: pattern.to_string(),
			},
		);
	}

	/// A table-level check written in SQL.
	pub fn sql_check(&mut self, name: &str, sql: &str) {
		self.table.checks.push(Check {
			name: name.to_string(),
			rule: CheckRule::Sql {
				sql: sql.to_string(),
			},
		});
	}

	/// A foreign key, when the column's type references a table. Required
	/// references restrict deletes by default; optional ones become `NULL`.
	/// A deferred key is checked when the transaction commits, so rows can
	/// point at each other while a transaction rearranges them.
	pub fn foreign_key<T: Column>(
		&mut self,
		column: &str,
		on_delete: Option<ReferentialAction>,
		deferred: bool,
	) {
		let Some(reference) = T::references() else {
			return;
		};
		let on_delete = on_delete.unwrap_or(if T::nullable() {
			ReferentialAction::SetNull
		} else {
			ReferentialAction::Restrict
		});
		let referenced: Vec<&str> = reference.columns.iter().map(String::as_str).collect();
		let mut foreign_key =
			ForeignKey::new(&self.table.name, &[column], reference.table, &referenced)
				.on_delete(on_delete);
		foreign_key.deferred = deferred;
		self.table.foreign_keys.push(foreign_key);
	}

	/// A full-text search index over `columns`, each with its weight.
	pub fn search(
		&mut self,
		language: &str,
		columns: &[(&str, leptoskit_db_schema::SearchWeight)],
	) {
		self.table.search = Some(leptoskit_db_schema::Search::new(
			language,
			columns
				.iter()
				.map(|(column, weight)| leptoskit_db_schema::SearchColumn::new(*column, *weight))
				.collect(),
		));
	}

	/// A spatial index over a geospatial column.
	pub fn geo_index(&mut self, column: &str) {
		self.table
			.geo_indexes
			.push(leptoskit_db_schema::GeoIndex::new(column));
	}

	/// A check that a geospatial column's shapes are valid.
	pub fn geo_valid(&mut self, column: &str) {
		self.check(
			column,
			"valid",
			CheckRule::ValidGeometry {
				column: column.to_string(),
			},
		);
	}

	/// A vector column's options: how it's measured, the index Postgres
	/// keeps, the text columns its vectors are made from, and whether
	/// clients see it. With sources, the table gains the column keeping
	/// their fingerprint, renamed with the vector column.
	pub fn vector(
		&mut self,
		column: &str,
		distance: Option<leptoskit_db_schema::VectorDistance>,
		index: Option<leptoskit_db_schema::VectorIndexKind>,
		sources: &[&str],
		public: bool,
	) {
		let mut options =
			leptoskit_db_schema::VectorOptions::new(column, distance).from_sources(sources);
		options.index = index;
		options.public = public;

		if !sources.is_empty() {
			let renamed_from = self
				.table
				.column(column)
				.and_then(|vector| vector.renamed_from.as_deref())
				.map(leptoskit_db_schema::source_column);
			self.table.columns.push(leptoskit_db_schema::Column {
				name: options.source_column(),
				ty: leptoskit_db_schema::ColumnType::TEXT,
				nullable: true,
				default: None,
				comment: Some(format!(
					"The fingerprint of the sources `{column}` was made from, which the database \
					 sets."
				)),
				renamed_from,
				generated: None,
			});
		}

		self.table.vectors.push(options);
	}

	pub fn on_account_merge(&mut self, strategy: leptoskit_db_schema::MergeStrategy) {
		self.table.on_account_merge = Some(strategy);
	}

	pub fn build(self) -> leptoskit_db_schema::Table {
		self.table
	}

	fn check(&mut self, column: &str, suffix: &str, rule: CheckRule) {
		self.table.checks.push(Check {
			name: constraint_name(&self.table.name, &[column], suffix),
			rule,
		});
	}
}

/// The same schema, keeping GeoPackage's metadata tables when `geopackage`,
/// and checked again: GeoPackage needs SQLite and one geometry column per
/// table.
pub fn with_geopackage(schema: Schema, geopackage: bool) -> Result<Schema, SchemaError> {
	let schema = schema.with_geopackage(geopackage);
	schema.validate()?;
	Ok(schema)
}

/// The environment variable naming the file `export_schema!` writes to.
pub const SCHEMA_OUT_ENV: &str = "LEPTOSKIT_DB_SCHEMA_OUT";

/// The environment variable naming the dialect `export_schema!` exports for.
pub const DIALECT_ENV: &str = "LEPTOSKIT_DB_DIALECT";

/// The environment variable that makes `export_schema!` serve the studio
/// instead of exporting the schema: the loopback address to listen on, such
/// as `127.0.0.1:0`. `leptoskit db studio` sets it.
pub const STUDIO_ENV: &str = "LEPTOSKIT_DB_STUDIO";

/// The environment variable naming the database the studio opens.
pub const STUDIO_DATABASE_ENV: &str = "LEPTOSKIT_DB_STUDIO_DATABASE";

/// The environment variable naming the app's migrations directory, for the
/// studio's migrations page.
pub const STUDIO_MIGRATIONS_ENV: &str = "LEPTOSKIT_DB_STUDIO_MIGRATIONS";

/// Writes every table's schema as `schema.json` when `leptoskit db` asks, by
/// setting [`SCHEMA_OUT_ENV`]; otherwise does nothing, so an ordinary
/// `cargo test` passes through it.
///
/// # Panics
///
/// When asked to export and the schema is invalid or can't be written: the
/// panic is the error `leptoskit db` shows.
#[doc(hidden)]
pub fn export_schema_if_requested() {
	if std::env::var_os(STUDIO_ENV).is_some() {
		serve_studio();
		return;
	}

	let Some(out) = std::env::var_os(SCHEMA_OUT_ENV) else {
		return;
	};
	let dialect = match std::env::var(DIALECT_ENV).as_deref() {
		Ok("sqlite") => Dialect::Sqlite,
		Ok("postgres") | Err(_) => Dialect::Postgres,
		Ok(other) => panic!("{DIALECT_ENV} is `{other}`; expected `postgres` or `sqlite`"),
	};

	let exported = match std::env::var(POLICIES_ENV).as_deref() {
		Ok("rls") => schema_with_row_security(dialect),
		Ok("orm") | Err(_) => schema(dialect),
		Ok(other) => panic!("{POLICIES_ENV} is `{other}`; expected `orm` or `rls`"),
	};
	let geopackage = match std::env::var(GEOPACKAGE_ENV).as_deref() {
		Ok("1") => true,
		Ok("0") | Err(_) => false,
		Ok(other) => panic!("{GEOPACKAGE_ENV} is `{other}`; expected `0` or `1`"),
	};
	let exported = exported.and_then(|schema| with_geopackage(schema, geopackage));

	match exported {
		Ok(schema) => {
			if let Err(error) = std::fs::write(&out, schema.to_json()) {
				panic!(
					"couldn't write the schema to {}: {error}",
					std::path::Path::new(&out).display()
				);
			}
		}
		Err(error) => panic!("{error}"),
	}
}

/// Serves the studio, when this build has it.
#[cfg(all(
	feature = "studio",
	debug_assertions,
	any(feature = "postgres", feature = "sqlite")
))]
fn serve_studio() {
	crate::studio::serve_from_env();
}

/// Says why this build has no studio.
///
/// # Panics
///
/// Always: the panic is the message `leptoskit db studio` shows.
#[cfg(not(all(
	feature = "studio",
	debug_assertions,
	any(feature = "postgres", feature = "sqlite")
)))]
fn serve_studio() {
	panic!(
		"this build has no studio: it needs the `studio` feature of leptoskit_db, a database \
		 driver, and debug assertions, which release builds turn off"
	);
}
