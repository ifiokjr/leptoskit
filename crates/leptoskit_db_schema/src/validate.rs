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
use crate::Predicate;
use crate::ReferentialAction;
use crate::Schema;
use crate::SchemaError;
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

impl Schema {
	/// Checks that every name is valid and unique, and that every key,
	/// index, and constraint refers to columns that exist with compatible
	/// types.
	pub fn validate(&self) -> Result<(), SchemaError> {
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

			self.validate_table(table, &mut problems);
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
				Some(_) => {}
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

	if let ColumnType::Array { element } = &column.ty {
		if matches!(**element, ColumnType::Array { .. }) {
			problems.push(location, "arrays of arrays aren't supported");
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
