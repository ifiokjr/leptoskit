//! Checks that a schema is internally consistent before anything is diffed or
//! rendered from it.

use std::collections::HashSet;
use std::fmt;
use std::fmt::Write;

use crate::Check;
use crate::CheckRule;
use crate::Column;
use crate::ColumnType;
use crate::DefaultValue;
use crate::Dialect;
use crate::ForeignKey;
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

			self.validate_geo_names(table, &mut flat_names, &mut problems);
			self.validate_table(table, &mut problems);
		}

		if self.geopackage && self.dialect != Dialect::Sqlite {
			problems.push(
				"schema",
				"GeoPackage metadata is for SQLite files; Postgres keeps PostGIS's own",
			);
		}

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
				validate_view_index(view, index, &location, &mut index_names, &mut problems);
			}
		}

		if let Err(cycle) = self.views_in_order() {
			let names: Vec<String> = cycle.iter().map(ToString::to_string).collect();
			problems.push(
				&names.join(", "),
				"these views read each other, so none can be created first; a view is taken to \
				 read every view its query names",
			);
		}

		problems.0
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

		for table in tables {
			for column in &table.columns {
				if matches!(column.default, Some(DefaultValue::Sql(_))) {
					problems.push(
						&format!("{}.{}", table.name, column.name),
						"a pack's column can't default to raw SQL, which is written for one \
						 dialect",
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
							"check `{}` is raw SQL, written for one dialect; a pack uses the \
							 portable rules",
							check.name
						),
					);
				}
			}
		}

		let mut found: Vec<(Dialect, Problem)> = Vec::new();

		for dialect in [Dialect::Postgres, Dialect::Sqlite] {
			if let Err(SchemaError::Invalid(invalid)) =
				Schema::new(dialect, tables.to_vec()).validate()
			{
				found.extend(invalid.into_iter().map(|problem| (dialect, problem)));
			}
		}

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

		if let Some(search) = &table.search {
			validate_search(self.dialect, table, search, problems);
		}

		validate_geo_indexes(table, problems);

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

		for column in &foreign_key.columns {
			problems.column_exists(table, &location, name, column);

			if table
				.column(column)
				.is_some_and(|column| column.generated.is_some())
			{
				problems.push(
					&location,
					format!(
						"foreign key `{name}` is on `{column}`, a generated column; link from a \
						 column the app writes"
					),
				);
			}
		}

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

		for (column, referenced_column) in foreign_key
			.columns
			.iter()
			.zip(&foreign_key.referenced_columns)
		{
			let Some(referenced_column) = referenced.column(referenced_column) else {
				problems.push(
					&location,
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
						&location,
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
						&location,
						format!(
							"foreign key `{name}` sets `{}` to NULL on delete, but the column \
							 isn't nullable",
							column.name
						),
					);
				}
			}
		}

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

fn validate_column(dialect: Dialect, column: &Column, location: &str, problems: &mut Problems) {
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

	if let Some(generated) = &column.generated {
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

	let Some(default) = &column.default else {
		return;
	};

	let fits = match default {
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
	};

	if !fits {
		problems.push(
			location,
			format!(
				"the default {default} doesn't fit the column's type, {}",
				column.ty
			),
		);
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

	if table.column(SEARCH_COLUMN).is_some() {
		problems.push(
			&location,
			format!("`{SEARCH_COLUMN}` holds a searched table's words, so no column may use it"),
		);
	}

	if table.primary_key.iter().any(|key| key == SEARCH_ROW) {
		problems.push(
			&location,
			format!(
				"on SQLite the search index numbers its rows in `{SEARCH_ROW}`, so the key can't \
				 use that name"
			),
		);
	}

	let mut seen = HashSet::new();

	for searched in &search.columns {
		if !seen.insert(searched.name.as_str()) {
			problems.push(&location, format!("`{}` is searched twice", searched.name));
		}

		match table.column(&searched.name) {
			None => {
				problems.push(
					&location,
					format!("searches `{}`, which isn't a column", searched.name),
				);
			}
			Some(column) if !matches!(column.ty, ColumnType::Text { .. }) => {
				problems.push(
					&location,
					format!(
						"searches `{}`, whose type is {}; only text is searched",
						searched.name, column.ty
					),
				);
			}
			Some(column) if column.generated.is_some() => {
				problems.push(
					&location,
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
