//! Compares two schemas and describes, step by step, how to turn one into the
//! other.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;

use crate::Check;
use crate::CheckRule;
use crate::Column;
use crate::Dialect;
use crate::ForeignKey;
use crate::Index;
use crate::Predicate;
use crate::Schema;
use crate::SchemaError;
use crate::Table;
use crate::TableName;

/// The steps that turn one schema into another, in the order they apply.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
	/// The dialect both schemas target.
	pub dialect: Dialect,
	/// The steps, ordered so that each one's dependencies come first.
	pub steps: Vec<Step>,
}

impl Plan {
	/// Whether the schemas were already the same.
	pub fn is_empty(&self) -> bool {
		self.steps.is_empty()
	}

	/// The steps that delete data. Generating a migration that contains any of
	/// these needs an explicit `--allow-destructive`.
	pub fn destructive_steps(&self) -> impl Iterator<Item = &Step> {
		self.steps
			.iter()
			.filter(|step| matches!(step.risk(), Risk::LosesData(_)))
	}
}

/// One change to a schema.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
	/// Create a table, with its key, indexes, foreign keys, and checks.
	CreateTable(Table),
	/// Drop a table and its data.
	DropTable(Table),
	/// Rename a table, keeping its data.
	RenameTable {
		/// The old name.
		from: TableName,
		/// The new name.
		to: TableName,
	},
	/// Add a column to an existing table.
	AddColumn {
		/// The table, by its new name.
		table: TableName,
		/// The new column.
		column: Column,
	},
	/// Drop a column and its data.
	DropColumn {
		/// The table, by its new name.
		table: TableName,
		/// The dropped column.
		column: Column,
	},
	/// Rename a column, keeping its data.
	RenameColumn {
		/// The table, by its new name.
		table: TableName,
		/// The old name.
		from: String,
		/// The new name.
		to: String,
	},
	/// Change a column's type, nullability, or default.
	AlterColumn {
		/// The table, by its new name.
		table: TableName,
		/// The column as it was, under its new name.
		from: Column,
		/// The column as it should be.
		to: Column,
	},
	/// Replace the primary key.
	SetPrimaryKey {
		/// The table, by its new name.
		table: TableName,
		/// The old key's columns, by their new names.
		from: Vec<String>,
		/// The new key's columns.
		to: Vec<String>,
	},
	/// Create an index on an existing table.
	CreateIndex {
		/// The table, by its new name.
		table: TableName,
		/// The new index.
		index: Index,
	},
	/// Drop an index.
	DropIndex {
		/// The table, by its new name.
		table: TableName,
		/// The dropped index.
		index: Index,
	},
	/// Rename an index whose definition is unchanged.
	RenameIndex {
		/// The table, by its new name.
		table: TableName,
		/// The old name.
		from: String,
		/// The index, under its new name.
		index: Index,
	},
	/// Add a foreign key to an existing table.
	AddForeignKey {
		/// The table, by its new name.
		table: TableName,
		/// The new foreign key.
		foreign_key: ForeignKey,
	},
	/// Drop a foreign key.
	DropForeignKey {
		/// The table, by its new name.
		table: TableName,
		/// The dropped foreign key.
		foreign_key: ForeignKey,
	},
	/// Add a check to an existing table.
	AddCheck {
		/// The table, by its new name.
		table: TableName,
		/// The new check.
		check: Check,
	},
	/// Drop a check.
	DropCheck {
		/// The table, by its new name.
		table: TableName,
		/// The dropped check.
		check: Check,
	},
	/// Rename a foreign key or check whose definition is unchanged.
	RenameConstraint {
		/// The table, by its new name.
		table: TableName,
		/// The old name.
		from: String,
		/// The new name.
		to: String,
	},
	/// Change a table's documentation.
	SetTableComment {
		/// The table, by its new name.
		table: TableName,
		/// The new comment, or none.
		comment: Option<String>,
	},
	/// Change a column's documentation.
	SetColumnComment {
		/// The table, by its new name.
		table: TableName,
		/// The column, by its new name.
		column: String,
		/// The new comment, or none.
		comment: Option<String>,
	},
}

/// How safe a step is to run against a database that already holds data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Risk {
	/// It always succeeds and keeps every value.
	Safe,
	/// It keeps every value, but fails if existing rows break a new rule.
	MayFail(String),
	/// It deletes or may change existing values.
	LosesData(String),
}

impl Step {
	/// The table the step changes, by its new name.
	pub fn table(&self) -> &TableName {
		match self {
			Self::CreateTable(table) | Self::DropTable(table) => &table.name,
			Self::RenameTable { to, .. } => to,
			Self::AddColumn { table, .. }
			| Self::DropColumn { table, .. }
			| Self::RenameColumn { table, .. }
			| Self::AlterColumn { table, .. }
			| Self::SetPrimaryKey { table, .. }
			| Self::CreateIndex { table, .. }
			| Self::DropIndex { table, .. }
			| Self::RenameIndex { table, .. }
			| Self::RenameConstraint { table, .. }
			| Self::AddForeignKey { table, .. }
			| Self::DropForeignKey { table, .. }
			| Self::AddCheck { table, .. }
			| Self::DropCheck { table, .. }
			| Self::SetTableComment { table, .. }
			| Self::SetColumnComment { table, .. } => table,
		}
	}

	/// How safe the step is against existing data.
	pub fn risk(&self) -> Risk {
		match self {
			Self::DropTable(table) => {
				Risk::LosesData(format!("every row in `{}` is deleted", table.name))
			}
			Self::DropColumn { table, column } => {
				Risk::LosesData(format!(
					"every value in `{table}.{}` is deleted",
					column.name
				))
			}
			Self::AlterColumn { table, from, to } => alter_risk(table, from, to),
			Self::SetPrimaryKey { table, .. } => {
				Risk::MayFail(format!(
					"fails if existing rows in `{table}` share a value of the new key"
				))
			}
			Self::CreateIndex { table, index } if index.unique => {
				Risk::MayFail(format!(
					"fails if existing rows in `{table}` share values of ({})",
					index.columns.join(", ")
				))
			}
			Self::AddForeignKey { table, foreign_key } => {
				Risk::MayFail(format!(
					"fails if existing rows in `{table}` reference missing `{}` rows",
					foreign_key.references
				))
			}
			Self::AddCheck { table, check } => {
				Risk::MayFail(format!(
					"fails if existing rows in `{table}` break `{}`",
					check.name
				))
			}
			_ => Risk::Safe,
		}
	}

	/// The order in which steps apply: dependencies before dependents, and
	/// destruction last.
	fn phase(&self) -> u8 {
		match self {
			Self::DropForeignKey { .. } => 0,
			Self::DropIndex { .. } => 1,
			Self::DropCheck { .. } => 2,
			Self::RenameTable { .. } => 3,
			Self::RenameColumn { .. }
			| Self::RenameIndex { .. }
			| Self::RenameConstraint { .. } => 4,
			Self::CreateTable(_) => 5,
			Self::AddColumn { .. } => 6,
			Self::AlterColumn { .. } => 7,
			Self::DropColumn { .. } => 8,
			Self::SetPrimaryKey { .. } => 9,
			Self::CreateIndex { .. } => 10,
			Self::AddCheck { .. } => 11,
			Self::AddForeignKey { .. } => 12,
			Self::SetTableComment { .. } | Self::SetColumnComment { .. } => 13,
			Self::DropTable(_) => 14,
		}
	}
}

impl fmt::Display for Step {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::CreateTable(table) => {
				write!(
					f,
					"+ table {} ({} columns)",
					table.name,
					table.columns.len()
				)
			}
			Self::DropTable(table) => write!(f, "- table {}", table.name),
			Self::RenameTable { from, to } => write!(f, "~ table {from} renamed to {to}"),
			Self::AddColumn { table, column } => {
				write!(f, "+ column {table}.{} ({})", column.name, column.ty)
			}
			Self::DropColumn { table, column } => write!(f, "- column {table}.{}", column.name),
			Self::RenameColumn { table, from, to } => {
				write!(f, "~ column {table}.{from} renamed to {to}")
			}
			Self::AlterColumn { table, from, to } => {
				write!(f, "~ column {table}.{}: ", to.name)?;
				let mut changes = Vec::new();

				if from.ty != to.ty {
					changes.push(format!("{} → {}", from.ty, to.ty));
				}

				if from.nullable != to.nullable {
					changes.push(
						if to.nullable {
							"now nullable"
						} else {
							"now NOT NULL"
						}
						.to_string(),
					);
				}

				if from.default != to.default {
					changes.push("new default".to_string());
				}

				f.write_str(&changes.join(", "))
			}
			Self::SetPrimaryKey { table, to, .. } => {
				write!(f, "~ primary key {table}({})", to.join(", "))
			}
			Self::CreateIndex { table, index } => {
				let kind = if index.unique {
					"unique index"
				} else {
					"index"
				};
				write!(f, "+ {kind} {table}({})", index.columns.join(", "))
			}
			Self::DropIndex { index, .. } => write!(f, "- index {}", index.name),
			Self::RenameIndex { from, index, .. } => {
				write!(f, "~ index {from} renamed to {}", index.name)
			}
			Self::RenameConstraint { from, to, .. } => {
				write!(f, "~ constraint {from} renamed to {to}")
			}
			Self::AddForeignKey { table, foreign_key } => {
				write!(
					f,
					"+ foreign key {table}({}) → {}({})",
					foreign_key.columns.join(", "),
					foreign_key.references,
					foreign_key.referenced_columns.join(", ")
				)
			}
			Self::DropForeignKey { foreign_key, .. } => {
				write!(f, "- foreign key {}", foreign_key.name)
			}
			Self::AddCheck { table, check } => write!(f, "+ check {table}: {}", check.name),
			Self::DropCheck { check, .. } => write!(f, "- check {}", check.name),
			Self::SetTableComment { table, .. } => write!(f, "~ comment on {table}"),
			Self::SetColumnComment { table, column, .. } => {
				write!(f, "~ comment on {table}.{column}")
			}
		}
	}
}

fn alter_risk(table: &TableName, from: &Column, to: &Column) -> Risk {
	if from.ty != to.ty && !from.ty.widens_to(&to.ty) {
		return Risk::LosesData(format!(
			"`{table}.{}` changes from {} to {}; values may not convert",
			to.name, from.ty, to.ty
		));
	}

	if from.nullable && !to.nullable && to.default.is_none() {
		return Risk::MayFail(format!("fails if any `{table}.{}` is NULL", to.name));
	}

	Risk::Safe
}

/// Why two schemas can't be compared.
#[derive(Debug, thiserror::Error)]
pub enum DiffError {
	/// One of the schemas contradicts itself.
	#[error(transparent)]
	Invalid(#[from] SchemaError),
	/// The schemas target different dialects.
	#[error(
		"the previous schema is for {previous} and the new one is for {target}; a project uses \
		 one dialect"
	)]
	DialectMismatch {
		/// The previous schema's dialect.
		previous: Dialect,
		/// The new schema's dialect.
		target: Dialect,
	},
	/// A rename hint names something the previous schema doesn't have.
	#[error("{location} says it was renamed from `{from}`, which the previous schema doesn't have")]
	UnknownRenameSource {
		/// The renamed table or column.
		location: String,
		/// The name in the hint.
		from: String,
	},
	/// A table or column appeared while a similar one disappeared, and nothing
	/// says whether that was a rename.
	#[error(
		"{added} is new and {} disappeared. If it was renamed, add #[db(renamed_from = \"…\")]; \
		 otherwise generate the migration without renames.",
		.candidates.join(" and ")
	)]
	AmbiguousRename {
		/// The new table or column.
		added: String,
		/// What it might have been renamed from.
		candidates: Vec<String>,
	},
	/// A `NOT NULL` column without a default can't be added to a table that may
	/// already have rows.
	#[error(
		"{table}.{column} is NOT NULL with no default, so existing rows have no value for it; \
		 give it a default or make it nullable"
	)]
	NotNullWithoutDefault {
		/// The table.
		table: TableName,
		/// The column.
		column: String,
	},
}

/// Decides whether something that appeared was renamed from something that
/// disappeared, when no hint says.
pub trait RenameResolver {
	/// Returns the table `added` was renamed from, if any. `candidates` are
	/// the tables that disappeared.
	fn table(
		&mut self,
		added: &Table,
		candidates: &[&Table],
	) -> Result<Option<TableName>, DiffError>;

	/// Returns the column `added` was renamed from, if any. `candidates` are
	/// the columns of the same type that disappeared from `table`.
	fn column(
		&mut self,
		table: &TableName,
		added: &Column,
		candidates: &[&Column],
	) -> Result<Option<String>, DiffError>;
}

/// Fails on every ambiguity, so the answer has to come from a
/// `renamed_from` hint. Use it where nobody can be asked, such as CI.
#[derive(Clone, Copy, Debug, Default)]
pub struct RequireHints;

impl RenameResolver for RequireHints {
	fn table(
		&mut self,
		added: &Table,
		candidates: &[&Table],
	) -> Result<Option<TableName>, DiffError> {
		Err(DiffError::AmbiguousRename {
			added: format!("table `{}`", added.name),
			candidates: candidates
				.iter()
				.map(|table| format!("`{}`", table.name))
				.collect(),
		})
	}

	fn column(
		&mut self,
		table: &TableName,
		added: &Column,
		candidates: &[&Column],
	) -> Result<Option<String>, DiffError> {
		Err(DiffError::AmbiguousRename {
			added: format!("column `{table}.{}`", added.name),
			candidates: candidates
				.iter()
				.map(|column| format!("`{table}.{}` ({})", column.name, column.ty))
				.collect(),
		})
	}
}

/// Treats every ambiguity as a new table or column, never a rename.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoRenames;

impl RenameResolver for NoRenames {
	fn table(&mut self, _: &Table, _: &[&Table]) -> Result<Option<TableName>, DiffError> {
		Ok(None)
	}

	fn column(
		&mut self,
		_: &TableName,
		_: &Column,
		_: &[&Column],
	) -> Result<Option<String>, DiffError> {
		Ok(None)
	}
}

/// Describes how to turn `previous` into `target`.
///
/// Renames come from `renamed_from` hints in `target`, then from `renames`
/// whenever something appeared while something similar disappeared.
pub fn diff(
	previous: &Schema,
	target: &Schema,
	renames: &mut dyn RenameResolver,
) -> Result<Plan, DiffError> {
	if previous.dialect != target.dialect {
		return Err(DiffError::DialectMismatch {
			previous: previous.dialect,
			target: target.dialect,
		});
	}

	previous.validate()?;
	target.validate()?;

	let pairs = match_tables(previous, target, renames)?;
	let mut steps = Vec::new();

	// The new name of every table that survives, by its old name.
	let table_renames: HashMap<&TableName, &TableName> = pairs
		.matched
		.iter()
		.map(|pair| (&pair.previous.name, &pair.target.name))
		.collect();
	let mut matched_tables = Vec::new();

	for pair in &pairs.matched {
		if pair.previous.name != pair.target.name {
			steps.push(Step::RenameTable {
				from: pair.previous.name.clone(),
				to: pair.target.name.clone(),
			});
		}

		let columns = match_columns(pair.previous, pair.target, renames)?;
		matched_tables.push((pair, columns));
	}

	// Column renames per table, by the table's new name, so that foreign keys
	// in other tables can be translated too.
	let column_renames: HashMap<&TableName, &HashMap<String, String>> = matched_tables
		.iter()
		.map(|(pair, columns)| (&pair.target.name, &columns.renames))
		.collect();
	let translate = Translate {
		tables: &table_renames,
		columns: &column_renames,
	};

	// Columns whose type changes: foreign keys through them are rebuilt.
	let mut retyped = HashSet::new();

	for (pair, columns) in &matched_tables {
		for (from, to) in &columns.matched {
			if from.ty != to.ty {
				retyped.insert((pair.target.name.clone(), to.name.clone()));
			}
		}
	}

	let rekeyed: HashSet<TableName> = matched_tables
		.iter()
		.filter(|(pair, columns)| {
			columns.key(&pair.previous.primary_key) != pair.target.primary_key
		})
		.map(|(pair, _)| pair.target.name.clone())
		.collect();

	for (pair, columns) in &matched_tables {
		diff_table(pair, columns, &translate, &retyped, &rekeyed, &mut steps)?;
	}

	for table in pairs.added {
		steps.push(Step::CreateTable(table.clone()));
	}

	for table in pairs.removed {
		steps.push(Step::DropTable(table.clone()));
	}

	// A stable sort keeps each phase in table and declaration order.
	steps.sort_by_key(Step::phase);

	Ok(Plan {
		dialect: target.dialect,
		steps,
	})
}

struct TablePair<'a> {
	previous: &'a Table,
	target: &'a Table,
}

struct TableMatches<'a> {
	matched: Vec<TablePair<'a>>,
	added: Vec<&'a Table>,
	removed: Vec<&'a Table>,
}

fn match_tables<'a>(
	previous: &'a Schema,
	target: &'a Schema,
	renames: &mut dyn RenameResolver,
) -> Result<TableMatches<'a>, DiffError> {
	let mut matched = Vec::new();
	let mut unmatched = Vec::new();
	let mut claimed = HashSet::new();

	for table in &target.tables {
		if let Some(previous_table) = previous.table(&table.name) {
			claimed.insert(&previous_table.name);
			matched.push(TablePair {
				previous: previous_table,
				target: table,
			});
			continue;
		}

		match &table.renamed_from {
			Some(from) if target.table(from).is_none() => {
				let previous_table = previous.table(from).ok_or_else(|| {
					DiffError::UnknownRenameSource {
						location: format!("table `{}`", table.name),
						from: from.to_string(),
					}
				})?;
				claimed.insert(&previous_table.name);
				matched.push(TablePair {
					previous: previous_table,
					target: table,
				});
			}
			_ => unmatched.push(table),
		}
	}

	let mut removed: Vec<&Table> = previous
		.tables
		.iter()
		.filter(|table| !claimed.contains(&table.name))
		.collect();
	let mut added = Vec::new();

	for table in unmatched {
		if removed.is_empty() {
			added.push(table);
			continue;
		}

		match renames.table(table, &removed)? {
			Some(from) => {
				let position = removed
					.iter()
					.position(|candidate| candidate.name == from)
					.ok_or_else(|| {
						DiffError::UnknownRenameSource {
							location: format!("table `{}`", table.name),
							from: from.to_string(),
						}
					})?;
				matched.push(TablePair {
					previous: removed.remove(position),
					target: table,
				});
			}
			None => added.push(table),
		}
	}

	Ok(TableMatches {
		matched,
		added,
		removed,
	})
}

struct ColumnMatches<'a> {
	/// Each surviving column, as it was and as it should be.
	matched: Vec<(&'a Column, &'a Column)>,
	added: Vec<&'a Column>,
	removed: Vec<&'a Column>,
	/// New names by old name, for the columns that were renamed.
	renames: HashMap<String, String>,
}

fn match_columns<'a>(
	previous: &'a Table,
	target: &'a Table,
	renames: &mut dyn RenameResolver,
) -> Result<ColumnMatches<'a>, DiffError> {
	let mut matched = Vec::new();
	let mut unmatched = Vec::new();
	let mut claimed = HashSet::new();

	for column in &target.columns {
		if let Some(previous_column) = previous.column(&column.name) {
			claimed.insert(previous_column.name.as_str());
			matched.push((previous_column, column));
			continue;
		}

		match &column.renamed_from {
			Some(from) if target.column(from).is_none() => {
				let previous_column = previous.column(from).ok_or_else(|| {
					DiffError::UnknownRenameSource {
						location: format!("column `{}.{}`", target.name, column.name),
						from: from.clone(),
					}
				})?;
				claimed.insert(previous_column.name.as_str());
				matched.push((previous_column, column));
			}
			_ => unmatched.push(column),
		}
	}

	let mut removed: Vec<&Column> = previous
		.columns
		.iter()
		.filter(|column| !claimed.contains(column.name.as_str()))
		.collect();
	let mut added = Vec::new();

	for column in unmatched {
		let candidates: Vec<&Column> = removed
			.iter()
			.copied()
			.filter(|candidate| candidate.ty == column.ty)
			.collect();

		if candidates.is_empty() {
			added.push(column);
			continue;
		}

		match renames.column(&target.name, column, &candidates)? {
			Some(from) => {
				let position = removed
					.iter()
					.position(|candidate| candidate.name == from)
					.ok_or_else(|| {
						DiffError::UnknownRenameSource {
							location: format!("column `{}.{}`", target.name, column.name),
							from: from.clone(),
						}
					})?;
				matched.push((removed.remove(position), column));
			}
			None => added.push(column),
		}
	}

	let renames = matched
		.iter()
		.filter(|(from, to)| from.name != to.name)
		.map(|(from, to)| (from.name.clone(), to.name.clone()))
		.collect();

	Ok(ColumnMatches {
		matched,
		added,
		removed,
		renames,
	})
}

/// Translates names in the previous schema into the target's, so that
/// constraints can be compared after renames.
struct Translate<'a> {
	tables: &'a HashMap<&'a TableName, &'a TableName>,
	columns: &'a HashMap<&'a TableName, &'a HashMap<String, String>>,
}

impl Translate<'_> {
	fn table(&self, name: &TableName) -> TableName {
		self.tables
			.get(name)
			.map_or_else(|| name.clone(), |&renamed| renamed.clone())
	}

	fn column(&self, table: &TableName, column: &str) -> String {
		self.columns
			.get(table)
			.and_then(|renames| renames.get(column))
			.map_or_else(|| column.to_string(), Clone::clone)
	}

	/// A foreign key from the previous schema, with this table's renamed
	/// columns and the referenced table's new names.
	fn foreign_key(&self, foreign_key: &ForeignKey, matches: &ColumnMatches<'_>) -> ForeignKey {
		let references = self.table(&foreign_key.references);

		ForeignKey {
			name: foreign_key.name.clone(),
			columns: matches.key(&foreign_key.columns),
			referenced_columns: foreign_key
				.referenced_columns
				.iter()
				.map(|column| self.column(&references, column))
				.collect(),
			references,
			on_delete: foreign_key.on_delete,
			deferred: foreign_key.deferred,
		}
	}
}

/// Translations of one table's own constraints through its column renames.
impl ColumnMatches<'_> {
	fn rename(&self, column: &str) -> String {
		self.renames
			.get(column)
			.cloned()
			.unwrap_or_else(|| column.to_string())
	}

	fn key(&self, columns: &[String]) -> Vec<String> {
		columns.iter().map(|column| self.rename(column)).collect()
	}

	fn index(&self, index: &Index) -> Index {
		Index {
			name: index.name.clone(),
			columns: self.key(&index.columns),
			unique: index.unique,
			predicate: index.predicate.as_ref().map(|predicate| {
				match predicate {
					Predicate::IsNull(column) => Predicate::IsNull(self.rename(column)),
					Predicate::IsNotNull(column) => Predicate::IsNotNull(self.rename(column)),
					Predicate::Sql(sql) => Predicate::Sql(sql.clone()),
				}
			}),
		}
	}

	fn check(&self, check: &Check) -> Check {
		let mut check = check.clone();

		match &mut check.rule {
			CheckRule::Range { column, .. }
			| CheckRule::Length { column, .. }
			| CheckRule::OneOf { column, .. }
			| CheckRule::Regex { column, .. } => *column = self.rename(column),
			CheckRule::Sql(_) => {}
		}

		check
	}
}

fn diff_table(
	pair: &TablePair<'_>,
	columns: &ColumnMatches<'_>,
	translate: &Translate<'_>,
	retyped: &HashSet<(TableName, String)>,
	rekeyed: &HashSet<TableName>,
	steps: &mut Vec<Step>,
) -> Result<(), DiffError> {
	let table = &pair.target.name;
	diff_columns(table, columns, steps)?;
	let previous_key = columns.key(&pair.previous.primary_key);

	if previous_key != pair.target.primary_key {
		steps.push(Step::SetPrimaryKey {
			table: table.clone(),
			from: previous_key,
			to: pair.target.primary_key.clone(),
		});
	}

	diff_constraints(pair, columns, translate, retyped, rekeyed, steps);

	if pair.previous.comment != pair.target.comment {
		steps.push(Step::SetTableComment {
			table: table.clone(),
			comment: pair.target.comment.clone(),
		});
	}

	Ok(())
}

fn diff_columns(
	table: &TableName,
	columns: &ColumnMatches<'_>,
	steps: &mut Vec<Step>,
) -> Result<(), DiffError> {
	for (from, to) in &columns.matched {
		if from.name != to.name {
			steps.push(Step::RenameColumn {
				table: table.clone(),
				from: from.name.clone(),
				to: to.name.clone(),
			});
		}

		if from.ty != to.ty || from.nullable != to.nullable || from.default != to.default {
			let mut from = (*from).clone();
			from.name.clone_from(&to.name);
			from.renamed_from = None;
			steps.push(Step::AlterColumn {
				table: table.clone(),
				from,
				to: without_hint(to),
			});
		}

		if from.comment != to.comment {
			steps.push(Step::SetColumnComment {
				table: table.clone(),
				column: to.name.clone(),
				comment: to.comment.clone(),
			});
		}
	}

	for column in &columns.added {
		if !column.nullable && column.default.is_none() {
			return Err(DiffError::NotNullWithoutDefault {
				table: table.clone(),
				column: column.name.clone(),
			});
		}

		steps.push(Step::AddColumn {
			table: table.clone(),
			column: without_hint(column),
		});
	}

	for column in &columns.removed {
		steps.push(Step::DropColumn {
			table: table.clone(),
			column: (*column).clone(),
		});
	}

	Ok(())
}

/// Compares indexes, foreign keys, and checks after the table's renames.
fn diff_constraints(
	pair: &TablePair<'_>,
	columns: &ColumnMatches<'_>,
	translate: &Translate<'_>,
	retyped: &HashSet<(TableName, String)>,
	rekeyed: &HashSet<TableName>,
	steps: &mut Vec<Step>,
) {
	let table = &pair.target.name;

	diff_named(
		pair.previous
			.indexes
			.iter()
			.map(|index| columns.index(index))
			.collect(),
		&pair.target.indexes,
		steps,
		|from, index| {
			Step::RenameIndex {
				table: table.clone(),
				from,
				index: index.clone(),
			}
		},
		|index| {
			Step::DropIndex {
				table: table.clone(),
				index,
			}
		},
		|index| {
			Step::CreateIndex {
				table: table.clone(),
				index,
			}
		},
	);

	// Foreign keys through a retyped column, or to a table whose key
	// changed, are rebuilt: Postgres can't change either while they exist.
	let touches_change = |foreign_key: &ForeignKey| {
		rekeyed.contains(&foreign_key.references)
			|| foreign_key
				.columns
				.iter()
				.any(|column| retyped.contains(&(table.clone(), column.clone())))
			|| foreign_key
				.referenced_columns
				.iter()
				.any(|column| retyped.contains(&(foreign_key.references.clone(), column.clone())))
	};
	let (rebuilt, previous_foreign_keys): (Vec<ForeignKey>, Vec<ForeignKey>) = pair
		.previous
		.foreign_keys
		.iter()
		.map(|foreign_key| translate.foreign_key(foreign_key, columns))
		.partition(|foreign_key| touches_change(foreign_key));
	let (readded, target_foreign_keys): (Vec<ForeignKey>, Vec<ForeignKey>) = pair
		.target
		.foreign_keys
		.iter()
		.cloned()
		.partition(|foreign_key| touches_change(foreign_key));

	for foreign_key in rebuilt {
		steps.push(Step::DropForeignKey {
			table: table.clone(),
			foreign_key,
		});
	}

	for foreign_key in readded {
		steps.push(Step::AddForeignKey {
			table: table.clone(),
			foreign_key,
		});
	}

	diff_named(
		previous_foreign_keys,
		&target_foreign_keys,
		steps,
		|from, foreign_key| {
			Step::RenameConstraint {
				table: table.clone(),
				from,
				to: foreign_key.name.clone(),
			}
		},
		|foreign_key| {
			Step::DropForeignKey {
				table: table.clone(),
				foreign_key,
			}
		},
		|foreign_key| {
			Step::AddForeignKey {
				table: table.clone(),
				foreign_key,
			}
		},
	);

	diff_named(
		pair.previous
			.checks
			.iter()
			.map(|check| columns.check(check))
			.collect(),
		&pair.target.checks,
		steps,
		|from, check| {
			Step::RenameConstraint {
				table: table.clone(),
				from,
				to: check.name.clone(),
			}
		},
		|check| {
			Step::DropCheck {
				table: table.clone(),
				check,
			}
		},
		|check| {
			Step::AddCheck {
				table: table.clone(),
				check,
			}
		},
	);
}

/// An index or constraint: unique by name within its table.
trait Named: Clone + PartialEq {
	fn name(&self) -> &str;

	/// The same definition under another name.
	fn named(&self, name: &str) -> Self;
}

impl Named for Index {
	fn name(&self) -> &str {
		&self.name
	}

	fn named(&self, name: &str) -> Self {
		Self {
			name: name.to_string(),
			..self.clone()
		}
	}
}

impl Named for ForeignKey {
	fn name(&self) -> &str {
		&self.name
	}

	fn named(&self, name: &str) -> Self {
		Self {
			name: name.to_string(),
			..self.clone()
		}
	}
}

impl Named for Check {
	fn name(&self) -> &str {
		&self.name
	}

	fn named(&self, name: &str) -> Self {
		Self {
			name: name.to_string(),
			..self.clone()
		}
	}
}

/// Compares named items. One whose definition is unchanged but whose name
/// changed, usually because its table was renamed, is renamed rather than
/// rebuilt; any other change is a drop and a create.
fn diff_named<T: Named>(
	previous: Vec<T>,
	target: &[T],
	steps: &mut Vec<Step>,
	rename: impl Fn(String, &T) -> Step,
	drop: impl Fn(T) -> Step,
	create: impl Fn(T) -> Step,
) {
	let mut created: Vec<&T> = target
		.iter()
		.filter(|item| !previous.contains(item))
		.collect();
	let mut dropped: Vec<T> = previous
		.into_iter()
		.filter(|item| !target.contains(item))
		.collect();

	created.retain(|item| {
		let Some(position) = dropped
			.iter()
			.position(|old| old.named(item.name()) == **item)
		else {
			return true;
		};

		let old = dropped.remove(position);
		steps.push(rename(old.name().to_string(), item));
		false
	});

	for item in dropped {
		steps.push(drop(item));
	}

	for item in created {
		steps.push(create(item.clone()));
	}
}

fn without_hint(column: &Column) -> Column {
	let mut column = column.clone();
	column.renamed_from = None;
	column
}
