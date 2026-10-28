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
	definition: fn() -> leptoskit_db_schema::Table,
}

impl TableEntry {
	pub const fn of<T: Table>() -> Self {
		Self {
			definition: T::definition,
		}
	}
}

inventory::collect!(TableEntry);

/// Every table linked into this program, as a schema for `dialect`.
///
/// `leptoskit db` builds a small program that calls this and diffs the result
/// against the latest migration's snapshot.
pub fn schema(dialect: Dialect) -> Result<Schema, SchemaError> {
	let tables = inventory::iter::<TableEntry>()
		.map(|entry| (entry.definition)())
		.collect();
	let schema = Schema::new(dialect, tables);
	schema.validate()?;
	Ok(schema)
}

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
	pub fn foreign_key<T: Column>(&mut self, column: &str, on_delete: Option<ReferentialAction>) {
		let Some(reference) = T::references() else {
			return;
		};
		let on_delete = on_delete.unwrap_or(if T::nullable() {
			ReferentialAction::SetNull
		} else {
			ReferentialAction::Restrict
		});
		let referenced: Vec<&str> = reference.columns.iter().map(String::as_str).collect();

		self.table.foreign_keys.push(
			ForeignKey::new(&self.table.name, &[column], reference.table, &referenced)
				.on_delete(on_delete),
		);
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
