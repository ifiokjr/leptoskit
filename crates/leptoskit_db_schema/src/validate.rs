//! Checks that a schema is internally consistent before anything is diffed or
//! rendered from it.

use std::collections::HashSet;
use std::fmt;
use std::fmt::Write;

use crate::BODY_QUOTE;
use crate::Check;
use crate::CheckRule;
use crate::Column;
use crate::ColumnType;
use crate::DefaultValue;
use crate::Dialect;
use crate::ForeignKey;
use crate::Function;
use crate::MAX_IDENTIFIER_LEN;
use crate::MAX_SRID;
use crate::PolicyCommand;
use crate::Predicate;
use crate::ReferentialAction;
use crate::SEARCH_COLUMN;
use crate::SEARCH_ROW;
use crate::Schema;
use crate::SchemaError;
use crate::Search;
use crate::Table;
use crate::TableName;
use crate::TriggerAction;
use crate::TriggerTiming;
use crate::VectorDistance;
use crate::VectorElement;
use crate::VectorIndexKind;
use crate::VectorOptions;

/// One inconsistency in a schema, located by table and, where it applies, by
/// column or constraint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
	/// Where the problem is, such as `users.handle` or `posts`.
	pub location: String,
	/// What is wrong, as a sentence.
	pub message: String,
}

impl fmt::Display for Problem {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}: {}", self.location, self.message)
	}
}

pub(crate) fn list(problems: &[Problem]) -> String {
	let mut text = String::new();

	for problem in problems {
		let _ = writeln!(text, "  - {problem}");
	}

	text
}

/// An index on a view: only a materialized view has rows to index, and only
/// over its own columns.
fn validate_view_index<'a>(
	view: &crate::View,
	index: &'a crate::Index,
	location: &str,
	index_names: &mut HashSet<&'a str>,
	problems: &mut Problems,
) {
	problems.identifier(location, "index name", &index.name);

	if !index_names.insert(index.name.as_str()) {
		problems.push(
			location,
			format!(
				"another table or view already has an index named `{}`",
				index.name
			),
		);
	}

	if !view.materialized {
		problems.push(
			location,
			format!(
				"index `{}`: only a materialized view keeps rows to index",
				index.name
			),
		);
	}

	if index.columns.is_empty() {
		problems.push(
			location,
			format!("index `{}` covers no columns", index.name),
		);
	}

	let predicate_column = match &index.predicate {
		Some(Predicate::IsNull(column) | Predicate::IsNotNull(column)) => Some(column),
		_ => None,
	};

	for column in index.columns.iter().chain(predicate_column) {
		if !view
			.columns
			.iter()
			.any(|candidate| &candidate.name == column)
		{
			problems.push(
				location,
				format!(
					"index `{}` names `{column}`, which isn't one of the view's columns",
					index.name
				),
			);
		}
	}
}

impl Schema {
	/// Checks that every name is valid and unique, and that every key,
	/// index, and constraint refers to columns that exist with compatible
	/// types.
	pub fn validate(&self) -> Result<(), SchemaError> {
		Problems(self.problems()).into_result()
	}

	/// Every way the schema contradicts itself, as [`Schema::validate`] finds
	/// them: empty when it's valid.
	pub fn problems(&self) -> Vec<Problem> {
		let mut problems = Problems::default();
		let mut flat_names = HashSet::new();
		let mut index_names = HashSet::new();

		self.table_problems(&mut flat_names, &mut index_names, &mut problems);

		if self.geopackage && self.dialect != Dialect::Sqlite {
			problems.push(
				"schema",
				"GeoPackage metadata is for SQLite files; Postgres keeps PostGIS's own",
			);
		}

		self.view_problems(&mut flat_names, &mut index_names, &mut problems);

		if let Err(cycle) = self.views_in_order() {
			let names: Vec<String> = cycle.iter().map(ToString::to_string).collect();
			problems.push(
				&names.join(", "),
				"these views read each other, so none can be created first; a view is taken to \
				 read every view its query names",
			);
		}

		self.validate_routines(&mut problems);
		problems.0
	}

	/// Every table's names, indexes, and own rules.
	fn table_problems<'a>(
		&'a self,
		flat_names: &mut HashSet<String>,
		index_names: &mut HashSet<&'a str>,
		problems: &mut Problems,
	) {
		for table in &self.tables {
			let location = table.name.to_string();

			if let Some(namespace) = &table.name.namespace {
				problems.identifier(&location, "namespace", namespace);
			}

			problems.identifier(&location, "table name", &table.name.name);

			if !flat_names.insert(table.name.flat()) {
				problems.push(
					&location,
					format!(
						"another table has the name `{}` once namespaces are flattened",
						table.name.flat()
					),
				);
			}

			for index in &table.indexes {
				if !index_names.insert(index.name.as_str()) {
					problems.push(
						&location,
						format!("another table already has an index named `{}`", index.name),
					);
				}
			}

			if table.search.is_some() {
				let index = Search::index_name(&table.name);

				for name in [
					Search::text_table(&table.name),
					Search::rows_table(&table.name),
				] {
					if !flat_names.insert(name.clone()) {
						problems.push(
							&location,
							format!(
								"the table's search index keeps its words in `{name}`, which is \
								 already another table's name"
							),
						);
					}
				}

				if table.indexes.iter().any(|other| other.name == index) {
					problems.push(
						&location,
						format!(
							"the table's search index is `{index}`, which is already another \
							 index's name"
						),
					);
				}
			}

			self.validate_geo_names(table, flat_names, problems);
			validate_vector_names(table, flat_names, problems);
			self.validate_table(table, problems);
		}
	}

	/// Every view's names, columns, query, indexes, and security.
	fn view_problems<'a>(
		&'a self,
		flat_names: &mut HashSet<String>,
		index_names: &mut HashSet<&'a str>,
		problems: &mut Problems,
	) {
		for view in &self.views {
			let location = view.name.to_string();

			if let Some(namespace) = &view.name.namespace {
				problems.identifier(&location, "namespace", namespace);
			}

			problems.identifier(&location, "view name", &view.name.name);

			if !flat_names.insert(view.name.flat()) {
				problems.push(
					&location,
					format!(
						"a table or another view has the name `{}` once namespaces are flattened",
						view.name.flat()
					),
				);
			}

			if view.columns.is_empty() {
				problems.push(&location, "a view needs at least one column");
			}

			let mut columns = HashSet::new();

			for column in &view.columns {
				problems.identifier(&location, "column name", &column.name);

				if !columns.insert(column.name.as_str()) {
					problems.push(
						&location,
						format!("the column `{}` is declared twice", column.name),
					);
				}

				if column.default.is_some() || column.generated.is_some() {
					problems.push(
						&format!("{location}.{}", column.name),
						"a view's column comes from its query, so it has no default and isn't \
						 generated",
					);
				}

				if self.dialect == Dialect::Sqlite && matches!(column.ty, ColumnType::Array { .. })
				{
					problems.push(
						&format!("{location}.{}", column.name),
						"SQLite has no array type; use Json<Vec<T>>",
					);
				}
			}

			if view.sql.sql(self.dialect).trim().is_empty() {
				problems.push(
					&location,
					format!("the view has no query for {}", self.dialect),
				);
			}

			for index in &view.indexes {
				validate_view_index(view, index, &location, index_names, problems);
			}

			self.validate_view_security(view, &location, problems);
		}
	}

	/// The key exists and is made of plain, non-null columns.
	fn validate_primary_key(table: &Table, problems: &mut Problems) {
		let location = table.name.to_string();

		if table.primary_key.is_empty() {
			problems.push(&location, "a table needs a primary key");
		}

		for key in &table.primary_key {
			match table.column(key) {
				None => {
					problems.push(
						&location,
						format!("the primary key names `{key}`, which isn't a column"),
					);
				}

				Some(column) if column.nullable => {
					problems.push(
						&location,
						format!("the primary key column `{key}` can't be nullable"),
					);
				}

				Some(column) if column.generated.is_some() => {
					problems.push(
						&location,
						format!("the primary key column `{key}` can't be generated"),
					);
				}

				Some(_) => {}
			}
		}
	}

	/// Row-level security: Postgres only, one policy per command, with the
	/// clauses that command needs.
	fn validate_row_security(&self, table: &Table, problems: &mut Problems) {
		let location = table.name.to_string();

		if let Some(security) = &table.row_security {
			if self.dialect == Dialect::Sqlite {
				problems.push(
					&location,
					"row-level security is Postgres's; on SQLite the database layer alone \
					 enforces policies",
				);
			}

			let mut commands = HashSet::new();

			for policy in &security.policies {
				if !commands.insert(policy.command) {
					problems.push(
						&location,
						format!(
							"two row-level security policies govern {}",
							policy.command.keyword()
						),
					);
				}

				let wants_using = policy.command != PolicyCommand::Insert;
				let wants_check = matches!(
					policy.command,
					PolicyCommand::Insert | PolicyCommand::Update
				);

				if policy.using.is_some() != wants_using || policy.check.is_some() != wants_check {
					problems.push(
						&location,
						format!(
							"the {} policy needs {}",
							policy.command.keyword(),
							match (wants_using, wants_check) {
								(true, true) => "both USING and WITH CHECK",
								(true, false) => "USING and no WITH CHECK",
								_ => "WITH CHECK and no USING",
							}
						),
					);
				}
			}
		}
	}

	/// Sequences and functions exist only on Postgres; every trigger is on a
	/// table of the schema and does what its dialect can.
	fn validate_routines(&self, problems: &mut Problems) {
		let postgres = self.dialect == Dialect::Postgres;
		let mut names = HashSet::new();

		self.validate_sequences(postgres, &mut names, problems);
		self.validate_functions(postgres, &mut names, problems);
		self.validate_triggers(postgres, problems);
	}

	/// Sequences exist only on Postgres, and no other object shares their
	/// names.
	fn validate_sequences<'a>(
		&'a self,
		postgres: bool,
		names: &mut HashSet<&'a TableName>,
		problems: &mut Problems,
	) {
		for sequence in &self.sequences {
			let location = format!("sequence {}", sequence.name);

			if !postgres {
				problems.push(&location, "SQLite has no sequences");
			}

			self.routine_name(&location, &sequence.name, names, problems);
		}
	}

	/// Functions exist only on Postgres, and their bodies match their
	/// checksums.
	fn validate_functions<'a>(
		&'a self,
		postgres: bool,
		names: &mut HashSet<&'a TableName>,
		problems: &mut Problems,
	) {
		for function in &self.functions {
			let location = format!("function {}", function.name);

			if !postgres {
				problems.push(&location, "SQLite has no functions");
			}

			self.routine_name(&location, &function.name, names, problems);

			if Function::checksum_of(&function.body) != function.checksum {
				problems.push(
					&location,
					"the body doesn't match its checksum; regenerate the schema rather than \
					 editing it",
				);
			}

			if function.body.contains(BODY_QUOTE) {
				problems.push(
					&location,
					format!("the body can't contain `{BODY_QUOTE}`, which quotes it"),
				);
			}
		}
	}

	/// Triggers point at their table and its columns, and do what the
	/// dialect can.
	fn validate_triggers(&self, postgres: bool, problems: &mut Problems) {
		let mut triggers = HashSet::new();

		for trigger in &self.triggers {
			let location = trigger.to_string();
			problems.identifier(&location, "trigger name", &trigger.name);

			// Postgres names a trigger within its table; SQLite, within the
			// database.
			let scope = if postgres {
				Some(trigger.table.clone())
			} else {
				None
			};

			if !triggers.insert((scope, trigger.name.as_str())) {
				problems.push(&location, "another trigger has the same name");
			}

			let Some(table) = self.table(&trigger.table) else {
				problems.push(&location, "its table isn't in the schema");
				continue;
			};

			for column in &trigger.columns {
				problems.column_exists(table, &location, &trigger.name, column);
			}

			let mut events = HashSet::new();

			if trigger.events.is_empty() {
				problems.push(&location, "a trigger needs at least one event");
			}

			for event in &trigger.events {
				if !events.insert(event) {
					problems.push(&location, format!("{} is listed twice", event.sql()));
				}
			}

			if trigger.deferred && (!postgres || trigger.timing != TriggerTiming::After) {
				problems.push(
					&location,
					"only a Postgres trigger that runs after the write can wait for the commit",
				);
			}

			match (&trigger.action, postgres) {
				(TriggerAction::Function { function, .. }, true) => {
					if self.function(function).is_none() {
						problems.push(
							&location,
							format!("it calls `{function}`, which isn't in the schema"),
						);
					}
				}
				(TriggerAction::Statements { .. }, false) => {
					if trigger.events.len() > 1 {
						problems.push(&location, "a SQLite trigger fires on one event");
					}
				}
				(TriggerAction::Function { .. }, false) => {
					problems.push(&location, "SQLite triggers run statements, not functions");
				}
				(TriggerAction::Statements { .. }, true) => {
					problems.push(&location, "Postgres triggers call a function");
				}
			}
		}
	}

	/// Checks a sequence's or function's name, which no table, view, or
	/// other routine may share.
	fn routine_name<'a>(
		&self,
		location: &str,
		name: &'a TableName,
		names: &mut HashSet<&'a TableName>,
		problems: &mut Problems,
	) {
		if let Some(namespace) = &name.namespace {
			problems.identifier(location, "namespace", namespace);
		}

		problems.identifier(location, "name", &name.name);

		if !names.insert(name) || self.table(name).is_some() || self.view(name).is_some() {
			problems.push(location, "another object in the schema has the same name");
		}
	}

	/// A view reads its tables with its reader's rights only on Postgres, and
	/// only when it's plain: a materialized view keeps the rows it read when
	/// it was last refreshed.
	fn validate_view_security(&self, view: &crate::View, location: &str, problems: &mut Problems) {
		if !view.security_invoker {
			return;
		}

		if self.dialect != Dialect::Postgres {
			problems.push(
				location,
				"only Postgres views read their tables with the reader's rights",
			);
		}

		if view.materialized {
			problems.push(
				location,
				"a materialized view keeps the rows it read when it was refreshed, so it can't \
				 read them with each reader's rights",
			);
		}
	}

	/// The names a table's spatial indexes give their companion tables and
	/// indexes are free, and with GeoPackage metadata, the table has one
	/// geometry column at most.
	fn validate_geo_names(
		&self,
		table: &Table,
		flat_names: &mut HashSet<String>,
		problems: &mut Problems,
	) {
		let location = table.name.to_string();

		for index in &table.geo_indexes {
			for name in [
				index.rtree_table(&table.name),
				index.rows_table(&table.name),
			] {
				if !flat_names.insert(name.clone()) {
					problems.push(
						&location,
						format!(
							"the spatial index on `{}` keeps its boxes in `{name}`, which is \
							 already another table's name",
							index.column
						),
					);
				}
			}

			let name = index.index_name(&table.name);

			if table.indexes.iter().any(|other| other.name == name) {
				problems.push(
					&location,
					format!(
						"the spatial index on `{}` is `{name}`, which is already another index's \
						 name",
						index.column
					),
				);
			}
		}

		if self.geopackage {
			let geo_columns: Vec<&str> = table
				.columns
				.iter()
				.filter(|column| column.ty.geo().is_some())
				.map(|column| column.name.as_str())
				.collect();

			if geo_columns.len() > 1 {
				problems.push(
					&location,
					format!(
						"GeoPackage allows one geometry column per table, and this one has {}",
						geo_columns.join(", ")
					),
				);
			}
		}
	}

	/// Checks that `tables` work on every dialect, as a schema pack's must,
	/// whichever one the app uses:
	///
	/// - **Both dialects.** They validate for Postgres and SQLite alike.
	/// - **Self-contained.** They link only to each other.
	/// - **No raw SQL.** Raw SQL defaults, checks, and generated columns are
	///   written for one dialect, so they're refused.
	///
	/// # Errors
	///
	/// [`SchemaError::Invalid`] with every problem, each naming the dialect
	/// it shows up on when only one does.
	pub fn validate_portable(tables: &[Table]) -> Result<(), SchemaError> {
		let mut problems = Problems::default();

		raw_sql_problems(tables, &mut problems);
		let found = dialect_problems(tables);
		merge_dialect_problems(&found, &mut problems);
		problems.into_result()
	}

	fn validate_table(&self, table: &Table, problems: &mut Problems) {
		let location = table.name.to_string();

		if table.columns.is_empty() {
			problems.push(&location, "a table needs at least one column");
		}

		let mut column_names = HashSet::new();

		for column in &table.columns {
			let column_location = format!("{location}.{}", column.name);
			problems.identifier(&column_location, "column name", &column.name);

			if !column_names.insert(column.name.as_str()) {
				problems.push(&column_location, "the column is declared twice");
			}

			validate_column(self.dialect, column, &column_location, problems);
		}

		Self::validate_primary_key(table, problems);

		if let Some(search) = &table.search {
			validate_search(self.dialect, table, search, problems);
		}

		validate_geo_indexes(table, problems);
		validate_vectors(self.dialect, table, problems);

		self.validate_row_security(table, problems);

		let mut constraint_names = HashSet::new();

		for index in &table.indexes {
			problems.identifier(&location, "index name", &index.name);

			if index.columns.is_empty() {
				problems.push(
					&location,
					format!("index `{}` covers no columns", index.name),
				);
			}

			for column in &index.columns {
				problems.column_exists(table, &location, &index.name, column);
			}

			match &index.predicate {
				Some(Predicate::IsNull(column) | Predicate::IsNotNull(column)) => {
					problems.column_exists(table, &location, &index.name, column);
				}
				Some(Predicate::Sql(_)) | None => {}
			}
		}

		for foreign_key in &table.foreign_keys {
			problems.identifier(&location, "foreign key name", &foreign_key.name);

			if !constraint_names.insert(foreign_key.name.as_str()) {
				problems.push(
					&location,
					format!("two constraints are named `{}`", foreign_key.name),
				);
			}

			self.validate_foreign_key(table, foreign_key, problems);
		}

		for check in &table.checks {
			problems.identifier(&location, "check name", &check.name);

			if !constraint_names.insert(check.name.as_str()) {
				problems.push(
					&location,
					format!("two constraints are named `{}`", check.name),
				);
			}

			validate_check(table, check, problems);
		}
	}

	fn validate_foreign_key(
		&self,
		table: &Table,
		foreign_key: &ForeignKey,
		problems: &mut Problems,
	) {
		let location = table.name.to_string();
		let name = &foreign_key.name;

		validate_foreign_key_columns(table, foreign_key, &location, problems);

		if foreign_key.columns.len() != foreign_key.referenced_columns.len() {
			problems.push(
				&location,
				format!("foreign key `{name}` has a different number of columns on each side"),
			);

			return;
		}

		let Some(referenced) = self.table(&foreign_key.references) else {
			problems.push(
				&location,
				format!(
					"foreign key `{name}` references `{}`, which isn't a table",
					foreign_key.references
				),
			);

			return;
		};

		validate_foreign_key_reference(table, foreign_key, referenced, &location, problems);

		let is_key = referenced.primary_key == foreign_key.referenced_columns
			|| referenced.indexes.iter().any(|index| {
				index.unique
					&& index.predicate.is_none()
					&& index.columns == foreign_key.referenced_columns
			});

		if !is_key {
			problems.push(
				&location,
				format!(
					"foreign key `{name}` must reference `{}`'s primary key or a unique index \
					 without a predicate",
					referenced.name
				),
			);
		}
	}
}

/// The key's own columns exist and aren't generated.
fn validate_foreign_key_columns(
	table: &Table,
	foreign_key: &ForeignKey,
	location: &str,
	problems: &mut Problems,
) {
	let name = &foreign_key.name;

	for column in &foreign_key.columns {
		problems.column_exists(table, location, name, column);

		if table
			.column(column)
			.is_some_and(|column| column.generated.is_some())
		{
			problems.push(
				location,
				format!(
					"foreign key `{name}` is on `{column}`, a generated column; link from a \
					 column the app writes"
				),
			);
		}
	}
}

/// Each column matches the type of the one it joins, and `set null` needs a
/// nullable column.
fn validate_foreign_key_reference(
	table: &Table,
	foreign_key: &ForeignKey,
	referenced: &Table,
	location: &str,
	problems: &mut Problems,
) {
	let name = &foreign_key.name;

	for (column, referenced_column) in foreign_key
		.columns
		.iter()
		.zip(&foreign_key.referenced_columns)
	{
		let Some(referenced_column) = referenced.column(referenced_column) else {
			problems.push(
				location,
				format!(
					"foreign key `{name}` references `{}.{referenced_column}`, which isn't a \
					 column",
					referenced.name
				),
			);
			continue;
		};

		if let Some(column) = table.column(column) {
			if column.ty != referenced_column.ty {
				problems.push(
					location,
					format!(
						"foreign key `{name}` joins `{}` ({}) to `{}.{}` ({}); the types must \
						 match",
						column.name,
						column.ty,
						referenced.name,
						referenced_column.name,
						referenced_column.ty
					),
				);
			}

			if foreign_key.on_delete == ReferentialAction::SetNull && !column.nullable {
				problems.push(
					location,
					format!(
						"foreign key `{name}` sets `{}` to NULL on delete, but the column isn't \
						 nullable",
						column.name
					),
				);
			}
		}
	}
}

/// A pack can't carry raw SQL: its dialect is chosen when it's used.
fn raw_sql_problems(tables: &[Table], problems: &mut Problems) {
	for table in tables {
		for column in &table.columns {
			if matches!(column.default, Some(DefaultValue::Sql(_))) {
				problems.push(
					&format!("{}.{}", table.name, column.name),
					"a pack's column can't default to raw SQL, which is written for one dialect",
				);
			}

			if column.generated.is_some() {
				problems.push(
					&format!("{}.{}", table.name, column.name),
					"a pack's column can't be generated: its expression is raw SQL",
				);
			}
		}

		for check in &table.checks {
			if matches!(check.rule, CheckRule::Sql { .. }) {
				problems.push(
					&table.name.to_string(),
					format!(
						"check `{}` is raw SQL, written for one dialect; a pack uses the portable \
						 rules",
						check.name
					),
				);
			}
		}
	}
}

/// The problems each dialect finds, in order.
fn dialect_problems(tables: &[Table]) -> Vec<(Dialect, Problem)> {
	let mut found: Vec<(Dialect, Problem)> = Vec::new();

	for dialect in [Dialect::Postgres, Dialect::Sqlite] {
		if let Err(SchemaError::Invalid(invalid)) = Schema::new(dialect, tables.to_vec()).validate()
		{
			found.extend(invalid.into_iter().map(|problem| (dialect, problem)));
		}
	}

	found
}

/// Keeps a problem every dialect shares once, and the rest with the dialect
/// that found it.
fn merge_dialect_problems(found: &[(Dialect, Problem)], problems: &mut Problems) {
	for (index, (dialect, problem)) in found.iter().enumerate() {
		let everywhere = found.iter().filter(|(_, other)| other == problem).count() > 1;

		if !everywhere {
			problems.push(
				&problem.location,
				format!("on {dialect}, {}", problem.message),
			);
		} else if found[..index].iter().all(|(_, other)| other != problem) {
			problems.0.push(problem.clone());
		}
	}
}

fn validate_column(dialect: Dialect, column: &Column, location: &str, problems: &mut Problems) {
	validate_column_type(dialect, column, location, problems);
	validate_generated_column(dialect, column, location, problems);

	let Some(default) = &column.default else {
		return;
	};

	if !default_fits(default, column) {
		problems.push(
			location,
			format!(
				"the default {default} doesn't fit the column's type, {}",
				column.ty
			),
		);
	}
}

/// Arrays, shapes, vectors, and coordinate systems this dialect can't store.
fn validate_column_type(
	dialect: Dialect,
	column: &Column,
	location: &str,
	problems: &mut Problems,
) {
	if dialect == Dialect::Sqlite && matches!(column.ty, ColumnType::Array { .. }) {
		problems.push(
			location,
			"SQLite has no array columns; store the list as JSON instead",
		);
	}

	if let ColumnType::Array { element } = &column.ty
		&& matches!(**element, ColumnType::Array { .. })
	{
		problems.push(location, "arrays of arrays aren't supported");
	}

	if let ColumnType::Geometry { srid, .. } = &column.ty
		&& !(1..=MAX_SRID).contains(srid)
	{
		problems.push(
			location,
			format!("SRID {srid} isn't a coordinate system; SRIDs run from 1 to {MAX_SRID}"),
		);
	}

	if let ColumnType::Array { element } = &column.ty
		&& element.geo().is_some()
	{
		problems.push(
			location,
			"arrays of shapes aren't supported; use a Multi shape",
		);
	}

	validate_vector_column(dialect, column, location, problems);
}

/// A generated column has no default, and an expression for each dialect.
fn validate_generated_column(
	dialect: Dialect,
	column: &Column,
	location: &str,
	problems: &mut Problems,
) {
	let Some(generated) = &column.generated else {
		return;
	};

	if column.default.is_some() {
		problems.push(
			location,
			"a generated column can't have a default: the database computes every value",
		);
	}

	if generated.sql(dialect).trim().is_empty() {
		problems.push(
			location,
			format!("the generated column has no expression on {dialect}"),
		);
	}
}

/// Whether `default` fits the column's type.
fn default_fits(default: &DefaultValue, column: &Column) -> bool {
	match default {
		DefaultValue::Bool(_) => column.ty == ColumnType::Bool,
		DefaultValue::Integer(_) => {
			matches!(
				column.ty,
				ColumnType::Int16
					| ColumnType::Int32
					| ColumnType::Int64
					| ColumnType::Float32
					| ColumnType::Float64
					| ColumnType::Decimal { .. }
			)
		}
		DefaultValue::Float(value) => {
			value.is_finite()
				&& matches!(
					column.ty,
					ColumnType::Float32 | ColumnType::Float64 | ColumnType::Decimal { .. }
				)
		}
		DefaultValue::Text(value) => {
			match column.ty {
				ColumnType::Text { max_len } => {
					max_len.is_none_or(|max| value.chars().count() <= max as usize)
				}
				_ => false,
			}
		}
		DefaultValue::Now => column.ty == ColumnType::Timestamp,
		DefaultValue::Sql(_) => true,
	}
}

/// A search index reads text columns the app writes, once each, in a
/// language the dialect has.
fn validate_search(dialect: Dialect, table: &Table, search: &Search, problems: &mut Problems) {
	let location = format!("{} search", table.name);
	problems.identifier(&location, "search language", &search.language);

	if dialect == Dialect::Sqlite
		&& search.language != Search::SIMPLE
		&& search.language != Search::ENGLISH
	{
		problems.push(
			&location,
			format!(
				"SQLite searches in `simple` or `english`, not `{}`; other languages need Postgres",
				search.language
			),
		);
	}

	if search.columns.is_empty() {
		problems.push(&location, "a search index needs at least one column");
	}

	validate_search_names(table, &location, problems);
	validate_searched_columns(table, search, &location, problems);
}

/// The search's own column names are reserved from the table's.
fn validate_search_names(table: &Table, location: &str, problems: &mut Problems) {
	if table.column(SEARCH_COLUMN).is_some() {
		problems.push(
			location,
			format!("`{SEARCH_COLUMN}` holds a searched table's words, so no column may use it"),
		);
	}

	if table.primary_key.iter().any(|key| key == SEARCH_ROW) {
		problems.push(
			location,
			format!(
				"on SQLite the search index numbers its rows in `{SEARCH_ROW}`, so the key can't \
				 use that name"
			),
		);
	}
}

/// Each searched column exists, holds text, and is listed once.
fn validate_searched_columns(
	table: &Table,
	search: &Search,
	location: &str,
	problems: &mut Problems,
) {
	let mut seen = HashSet::new();

	for searched in &search.columns {
		if !seen.insert(searched.name.as_str()) {
			problems.push(location, format!("`{}` is searched twice", searched.name));
		}

		match table.column(&searched.name) {
			None => {
				problems.push(
					location,
					format!("searches `{}`, which isn't a column", searched.name),
				);
			}

			Some(column) if !matches!(column.ty, ColumnType::Text { .. }) => {
				problems.push(
					location,
					format!(
						"searches `{}`, whose type is {}; only text is searched",
						searched.name, column.ty
					),
				);
			}

			Some(column) if column.generated.is_some() => {
				problems.push(
					location,
					format!(
						"searches `{}`, a generated column; search the columns it's computed from",
						searched.name
					),
				);
			}

			Some(_) => {}
		}
	}
}

/// The most dimensions pgvector's `vector` and `halfvec` hold.
pub const MAX_VECTOR_DIMENSIONS: u32 = 16_000;

/// The most dimensions a sqlite-vec `vec0` table holds.
pub const MAX_SQLITE_VECTOR_DIMENSIONS: u32 = 8_192;

/// The most dimensions pgvector's `sparsevec` holds.
pub const MAX_SPARSE_DIMENSIONS: u32 = 1_000_000_000;

/// A vector column's type fits the dialect: dimensions in range, bits a
/// multiple of 8, and no sparse vectors on SQLite.
fn validate_vector_column(
	dialect: Dialect,
	column: &Column,
	location: &str,
	problems: &mut Problems,
) {
	if let ColumnType::Array { element } = &column.ty
		&& element.vector().is_some()
	{
		problems.push(location, "arrays of vectors aren't supported");
	}

	let Some((kind, dimensions)) = column.ty.vector() else {
		return;
	};

	let most = match (dialect, kind) {
		(Dialect::Sqlite, VectorElement::Sparse) => {
			problems.push(
				location,
				"sparse vectors need Postgres: sqlite-vec has no sparse type; use `Vector<N>` on \
				 SQLite",
			);
			return;
		}
		(Dialect::Sqlite, VectorElement::Bit) => MAX_SQLITE_VECTOR_DIMENSIONS * 8,
		(Dialect::Sqlite, _) => MAX_SQLITE_VECTOR_DIMENSIONS,
		(Dialect::Postgres, VectorElement::Sparse) => MAX_SPARSE_DIMENSIONS,
		(Dialect::Postgres, VectorElement::Bit) => u32::MAX,
		(Dialect::Postgres, _) => MAX_VECTOR_DIMENSIONS,
	};

	if !(1..=most).contains(&dimensions) {
		problems.push(
			location,
			format!(
				"a {} on {dialect} has 1 to {most} dimensions, not {dimensions}",
				kind.postgres_name()
			),
		);
	}

	if kind == VectorElement::Bit && dimensions % 8 != 0 {
		problems.push(
			location,
			format!("a bit vector's dimensions are a multiple of 8, not {dimensions}"),
		);
	}

	if column.generated.is_some() {
		problems.push(
			location,
			"a vector column can't be generated; store the vectors the app makes",
		);
	}
}

/// The most dimensions pgvector indexes for a kind of vector.
fn most_indexed(kind: VectorElement) -> u32 {
	match kind {
		VectorElement::Float32 => 2_000,
		VectorElement::Float16 => 4_000,
		VectorElement::Bit => 64_000,
		VectorElement::Sparse => MAX_SPARSE_DIMENSIONS,
	}
}

/// Each vector column's options name a vector column once, with a distance
/// that measures its kind, an index pgvector can build, and text sources
/// with the column that keeps their fingerprint.
fn validate_vectors(dialect: Dialect, table: &Table, problems: &mut Problems) {
	let mut seen = HashSet::new();

	for vector in &table.vectors {
		let location = format!("{}.{}", table.name, vector.column);

		if !seen.insert(vector.column.as_str()) {
			problems.push(&location, "the column's vector options are declared twice");
		}

		let Some(column) = table.column(&vector.column) else {
			problems.push(
				&table.name.to_string(),
				format!(
					"vector options name `{}`, which isn't a column",
					vector.column
				),
			);
			continue;
		};
		let Some((kind, dimensions)) = column.ty.vector() else {
			problems.push(
				&location,
				format!(
					"`vector` options are for vector columns, and this one's type is {}",
					column.ty
				),
			);
			continue;
		};

		if let Some(distance) = vector.distance
			&& !kind.measures(distance)
		{
			problems.push(
				&location,
				format!(
					"{} distance doesn't measure a {}; {}",
					distance,
					kind.postgres_name(),
					if kind == VectorElement::Bit {
						"bits are measured by `hamming` or `jaccard`"
					} else {
						"floats are measured by `l2`, `cosine`, `inner_product`, or `l1`"
					}
				),
			);
		}

		if let Some(index) = vector.index {
			validate_vector_index(
				dialect, vector, index, kind, dimensions, &location, problems,
			);
		}

		if !vector.sources.is_empty() {
			validate_vector_sources(table, vector, &location, problems);
		}
	}

	let indexed_on_sqlite = table
		.vectors
		.iter()
		.any(|vector| vector.is_indexed(Dialect::Sqlite));

	if dialect == Dialect::Sqlite
		&& indexed_on_sqlite
		&& table.primary_key.iter().any(|key| key == SEARCH_ROW)
	{
		problems.push(
			&table.name.to_string(),
			format!(
				"on SQLite a vector column's index numbers its rows in `{SEARCH_ROW}`, so the key \
				 can't use that name"
			),
		);
	}
}

/// An index pgvector can build: a distance to order it by, the operator
/// class IVFFlat has for it, and few enough dimensions.
fn validate_vector_index(
	dialect: Dialect,
	vector: &VectorOptions,
	index: VectorIndexKind,
	kind: VectorElement,
	dimensions: u32,
	location: &str,
	problems: &mut Problems,
) {
	let Some(distance) = vector.distance else {
		problems.push(
			location,
			format!(
				"an `{}` index orders rows by a distance; add `distance = …`",
				index.name()
			),
		);

		return;
	};

	// SQLite searches exactly, so an index there is only kept for Postgres.
	if dialect == Dialect::Sqlite {
		return;
	}

	if index == VectorIndexKind::IvfFlat {
		let refused = match (kind, distance) {
			(VectorElement::Sparse, _) => Some("sparse vectors"),
			(_, VectorDistance::L1) => Some("`l1` distance"),
			(VectorElement::Bit, VectorDistance::Jaccard) => Some("`jaccard` distance"),
			_ => None,
		};

		if let Some(refused) = refused {
			problems.push(
				location,
				format!("pgvector's `ivfflat` has no index for {refused}; use `index = hnsw`"),
			);
		}
	}

	let most = most_indexed(kind);

	if kind != VectorElement::Sparse && dimensions > most {
		problems.push(
			location,
			format!(
				"pgvector indexes a {} of at most {most} dimensions, and this one has \
				 {dimensions}; store a `HalfVector` or `BitVector`, or search it without an index",
				kind.postgres_name()
			),
		);
	}
}

/// Sources are text columns, each once and none the vector itself, and the
/// table has the nullable text column that keeps their fingerprint.
fn validate_vector_sources(
	table: &Table,
	vector: &VectorOptions,
	location: &str,
	problems: &mut Problems,
) {
	let mut seen = HashSet::new();

	for source in &vector.sources {
		if !seen.insert(source.as_str()) {
			problems.push(location, format!("`{source}` is a source twice"));
		}

		match table.column(source) {
			None => {
				problems.push(
					location,
					format!("the vector is made from `{source}`, which isn't a column"),
				);
			}

			Some(column) if !matches!(column.ty, ColumnType::Text { .. }) => {
				problems.push(
					location,
					format!(
						"the vector is made from `{source}`, whose type is {}; sources are text",
						column.ty
					),
				);
			}

			Some(_) => {}
		}
	}

	let name = vector.source_column();

	if name.len() > MAX_IDENTIFIER_LEN {
		problems.push(
			location,
			format!(
				"the column keeping the sources' fingerprint, `{name}`, is longer than the \
				 {MAX_IDENTIFIER_LEN}-byte limit; shorten the vector column's name"
			),
		);
	}

	let fits = table.column(&name).is_some_and(|column| {
		column.nullable
			&& column.generated.is_none()
			&& matches!(column.ty, ColumnType::Text { max_len: None })
	});

	if !fits {
		problems.push(
			location,
			format!(
				"a vector made from sources keeps their fingerprint in a nullable text column, \
				 `{name}`"
			),
		);
	}
}

/// The names a table's vector indexes give their companion tables, and
/// pgvector's index, are free.
fn validate_vector_names(table: &Table, flat_names: &mut HashSet<String>, problems: &mut Problems) {
	let location = table.name.to_string();

	for vector in &table.vectors {
		if vector.is_indexed(Dialect::Sqlite) {
			for name in [
				vector.vec_table(&table.name),
				vector.rows_table(&table.name),
			] {
				if !flat_names.insert(name.clone()) {
					problems.push(
						&location,
						format!(
							"the index on `{}` keeps its vectors in `{name}`, which is already \
							 another table's name",
							vector.column
						),
					);
				}
			}
		}

		if vector.index.is_some() {
			let name = vector.index_name(&table.name);

			if table.indexes.iter().any(|other| other.name == name) {
				problems.push(
					&location,
					format!(
						"the index on `{}` is `{name}`, which is already another index's name",
						vector.column
					),
				);
			}
		}
	}
}

/// A spatial index covers a geospatial column the app writes, once.
fn validate_geo_indexes(table: &Table, problems: &mut Problems) {
	let location = table.name.to_string();
	let mut seen = HashSet::new();

	if !table.geo_indexes.is_empty() && table.primary_key.iter().any(|key| key == SEARCH_ROW) {
		problems.push(
			&location,
			format!(
				"on SQLite a spatial index numbers its rows in `{SEARCH_ROW}`, so the key can't \
				 use that name"
			),
		);
	}

	for index in &table.geo_indexes {
		if !seen.insert(index.column.as_str()) {
			problems.push(
				&location,
				format!("`{}` has two spatial indexes", index.column),
			);
		}

		match table.column(&index.column) {
			None => {
				problems.push(
					&location,
					format!(
						"a spatial index covers `{}`, which isn't a column",
						index.column
					),
				);
			}

			Some(column) if column.ty.geo().is_none() => {
				problems.push(
					&location,
					format!(
						"a spatial index covers `{}`, whose type is {}; only shapes are indexed \
						 spatially",
						index.column, column.ty
					),
				);
			}

			Some(column) if column.generated.is_some() => {
				problems.push(
					&location,
					format!(
						"a spatial index covers `{}`, a generated column; index a column the app \
						 writes",
						index.column
					),
				);
			}

			Some(_) => {}
		}
	}
}

fn validate_check(table: &Table, check: &Check, problems: &mut Problems) {
	let location = table.name.to_string();
	let Some(column_name) = check.rule.column() else {
		return;
	};
	let Some(column) = table.column(column_name) else {
		problems.push(
			&location,
			format!(
				"check `{}` names `{column_name}`, which isn't a column",
				check.name
			),
		);

		return;
	};

	let fits = match &check.rule {
		CheckRule::Range { min, max, .. } => {
			min.is_none_or(f64::is_finite)
				&& max.is_none_or(f64::is_finite)
				&& matches!(
					column.ty,
					ColumnType::Int16
						| ColumnType::Int32
						| ColumnType::Int64
						| ColumnType::Float32
						| ColumnType::Float64
						| ColumnType::Decimal { .. }
				)
		}
		CheckRule::Length { .. } | CheckRule::OneOf { .. } | CheckRule::Regex { .. } => {
			matches!(column.ty, ColumnType::Text { .. })
		}
		CheckRule::ValidGeometry { .. } => column.ty.geo().is_some(),
		CheckRule::Sql { .. } => true,
	};

	if !fits {
		problems.push(
			&location,
			format!(
				"check `{}` doesn't apply to `{column_name}`, whose type is {}",
				check.name, column.ty
			),
		);
	}
}

#[derive(Default)]
struct Problems(Vec<Problem>);

impl Problems {
	fn push(&mut self, location: &str, message: impl Into<String>) {
		self.0.push(Problem {
			location: location.to_string(),
			message: message.into(),
		});
	}

	/// Identifiers are lowercase snake case, so Postgres never folds them and
	/// every name reads the same in both dialects.
	fn identifier(&mut self, location: &str, what: &str, identifier: &str) {
		let mut chars = identifier.chars();
		let starts_well = chars
			.next()
			.is_some_and(|first| first.is_ascii_lowercase() || first == '_');
		let rest_is_snake =
			chars.all(|char| char.is_ascii_lowercase() || char.is_ascii_digit() || char == '_');

		if !starts_well || !rest_is_snake {
			self.push(
				location,
				format!(
					"the {what} `{identifier}` must be lowercase snake case (a-z, 0-9, _), \
					 starting with a letter or _"
				),
			);
		}

		if identifier.len() > MAX_IDENTIFIER_LEN {
			self.push(
				location,
				format!(
					"the {what} `{identifier}` is longer than Postgres's \
					 {MAX_IDENTIFIER_LEN}-byte limit"
				),
			);
		}
	}

	fn column_exists(&mut self, table: &Table, location: &str, constraint: &str, column: &str) {
		if table.column(column).is_none() {
			self.push(
				location,
				format!("`{constraint}` names `{column}`, which isn't a column"),
			);
		}
	}

	fn into_result(self) -> Result<(), SchemaError> {
		if self.0.is_empty() {
			Ok(())
		} else {
			Err(SchemaError::Invalid(self.0))
		}
	}
}
