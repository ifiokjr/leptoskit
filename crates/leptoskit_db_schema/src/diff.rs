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
use crate::Function;
use crate::GeoIndex;
use crate::GeoPackageLayer;
use crate::Index;
use crate::Predicate;
use crate::Publication;
use crate::RowSecurity;
use crate::Schema;
use crate::SchemaError;
use crate::Search;
use crate::Sequence;
use crate::Table;
use crate::TableName;
use crate::Trigger;
use crate::VectorOptions;
use crate::View;

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
	/// Create a table's full-text search index, after every other change to
	/// the table, and index the rows it already has.
	CreateSearch {
		/// The table, by its new name.
		table: TableName,
		/// The index.
		search: Search,
	},
	/// Drop a table's full-text search index, before any other change to the
	/// table. Its words are derived from the table's own text, so nothing is
	/// lost; a changed index is dropped and created again.
	DropSearch {
		/// The table, by its new name.
		table: TableName,
		/// The index as it was.
		search: Search,
	},
	/// Create a table's row-level security policies, after every other
	/// change to the tables, and turn row-level security on for it.
	CreateRowSecurity {
		/// The table, by its new name.
		table: TableName,
		/// The policies.
		security: RowSecurity,
	},
	/// Drop a table's row-level security policies, before any change to the
	/// tables, since Postgres won't change a column a policy reads. Row-level
	/// security is turned off unless the plan creates policies again.
	DropRowSecurity {
		/// The table, by its new name.
		table: TableName,
		/// The policies as they were.
		security: RowSecurity,
	},
	/// Create a spatial index over a geospatial column, after every other
	/// change to the table, and index the rows it already has.
	CreateGeoIndex {
		/// The table, by its new name.
		table: TableName,
		/// The index.
		index: GeoIndex,
	},
	/// Drop a spatial index, before any other change to the table. It's
	/// derived from the table's own shapes, so nothing is lost; a changed
	/// index is dropped and created again.
	DropGeoIndex {
		/// The table, by its new name.
		table: TableName,
		/// The index as it was.
		index: GeoIndex,
	},
	/// Index a vector column for nearest-neighbour search, after every other
	/// change to the table, and index the rows it already has: pgvector's
	/// index on Postgres, a sqlite-vec `vec0` table on SQLite.
	CreateVectorIndex {
		/// The table, by its new name.
		table: TableName,
		/// The vector column's options.
		vector: VectorOptions,
	},
	/// Drop a vector column's index, before any other change to the table.
	/// It's derived from the column, so nothing is lost; a changed index is
	/// dropped and created again.
	DropVectorIndex {
		/// The table, by its new name.
		table: TableName,
		/// The vector column's options as they were.
		vector: VectorOptions,
	},
	/// Start recording which sources each row's vector was made from: the
	/// triggers that set its fingerprint, after every other change to the
	/// table. Vectors already there are taken as made from the row's
	/// sources.
	CreateVectorSource {
		/// The table, by its new name.
		table: TableName,
		/// The vector column's options.
		vector: VectorOptions,
	},
	/// Stop recording a vector column's sources, before any other change to
	/// the table. The fingerprints already recorded stay in their column.
	DropVectorSource {
		/// The table, by its new name.
		table: TableName,
		/// The vector column's options as they were.
		vector: VectorOptions,
	},
	/// Register a table's geometry column in GeoPackage's metadata tables,
	/// creating them first if they're missing. SQLite only.
	RegisterGeoPackage {
		/// The table, by its new name.
		table: TableName,
		/// The registered column.
		layer: GeoPackageLayer,
	},
	/// Remove a table's registration from GeoPackage's metadata tables, before
	/// any other change. SQLite only.
	UnregisterGeoPackage {
		/// The table, by its new name.
		table: TableName,
		/// The registration as it was.
		layer: GeoPackageLayer,
	},
	/// Create a view, after every table change.
	CreateView(View),
	/// Drop a view, before any table change. A changed view is dropped and
	/// created again, and so is every view when a table it may read
	/// changes: a view's SQL isn't parsed, so what it reads isn't known.
	DropView(View),
	/// Create a sequence, Postgres only.
	CreateSequence(Sequence),
	/// Drop a sequence, after the tables and functions that use it.
	DropSequence(Sequence),
	/// Create a function, before any trigger that calls it.
	CreateFunction(Function),
	/// Replace a function's definition in place, as `CREATE OR REPLACE`.
	ReplaceFunction {
		/// The function as it was.
		from: Function,
		/// The function as it should be.
		to: Function,
	},
	/// Drop a function, after every trigger that called it.
	DropFunction(Function),
	/// Create a trigger, after every table, function, and view change.
	CreateTrigger(Trigger),
	/// Drop a trigger, before any table change. A changed trigger is dropped
	/// and created again; so is every trigger on a SQLite table the
	/// migration alters, since rebuilding a table drops its triggers.
	DropTrigger(Trigger),
	/// Postgres only: rebuild a table that becomes partitioned, or stops
	/// being, copying its rows across. The table's other changes happen in
	/// the rebuild: it's created as `to` says.
	Repartition {
		/// The table as it was, by its name before the step.
		from: Table,
		/// The table as it should be.
		to: Table,
		/// The columns whose values are copied: each column's previous name
		/// and its new one.
		columns: Vec<(String, String)>,
	},
	/// Postgres only: create a publication, after every table change, and
	/// give each of its tables `REPLICA IDENTITY FULL`.
	CreatePublication {
		/// The publication's name, as a step names what it changes.
		name: TableName,
		/// The publication.
		publication: Publication,
	},
	/// Postgres only: change which tables a publication covers, or what it
	/// says about them, after every table change.
	AlterPublication {
		/// The publication's name.
		name: TableName,
		/// The publication as it was, with its tables' new names.
		from: Publication,
		/// The publication as it should be.
		to: Publication,
	},
	/// Postgres only: drop a publication, before any table change, and give
	/// its tables their default replica identity back.
	DropPublication {
		/// The publication's name.
		name: TableName,
		/// The publication.
		publication: Publication,
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
			| Self::SetColumnComment { table, .. }
			| Self::CreateSearch { table, .. }
			| Self::DropSearch { table, .. }
			| Self::CreateRowSecurity { table, .. }
			| Self::DropRowSecurity { table, .. }
			| Self::CreateGeoIndex { table, .. }
			| Self::DropGeoIndex { table, .. }
			| Self::CreateVectorIndex { table, .. }
			| Self::DropVectorIndex { table, .. }
			| Self::CreateVectorSource { table, .. }
			| Self::DropVectorSource { table, .. }
			| Self::RegisterGeoPackage { table, .. }
			| Self::UnregisterGeoPackage { table, .. } => table,
			Self::CreateView(view) | Self::DropView(view) => &view.name,
			Self::CreateSequence(sequence) | Self::DropSequence(sequence) => &sequence.name,
			Self::CreateFunction(function)
			| Self::DropFunction(function)
			| Self::ReplaceFunction { to: function, .. } => &function.name,
			Self::CreateTrigger(trigger) | Self::DropTrigger(trigger) => &trigger.table,
			Self::Repartition { to, .. } => &to.name,
			Self::CreatePublication { name, .. }
			| Self::AlterPublication { name, .. }
			| Self::DropPublication { name, .. } => name,
		}
	}

	/// How safe the step is against existing data.
	pub fn risk(&self) -> Risk {
		match self {
			Self::DropTable(table) => {
				Risk::LosesData(format!("every row in `{}` is deleted", table.name))
			}
			// The database computed every value of a generated column from
			// the row, so dropping one deletes nothing it couldn't recompute.
			Self::DropColumn { column, .. } if column.generated.is_some() => Risk::Safe,
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
			Self::Repartition { to, .. } => {
				Risk::MayFail(format!(
					"copies every row of `{}` into a new table, holding the old one locked \
					 meanwhile",
					to.name
				))
			}
			_ => Risk::Safe,
		}
	}

	/// The order in which steps apply: dependencies before dependents, and
	/// destruction last.
	fn phase(&self) -> u8 {
		match self {
			Self::DropTrigger(_) | Self::DropPublication { .. } => 0,
			Self::DropView(_)
			| Self::DropSearch { .. }
			| Self::DropRowSecurity { .. }
			| Self::DropGeoIndex { .. }
			| Self::DropVectorIndex { .. }
			| Self::DropVectorSource { .. }
			| Self::UnregisterGeoPackage { .. } => 1,
			Self::DropForeignKey { .. } => 2,
			Self::DropIndex { .. } => 3,
			Self::DropCheck { .. } => 4,
			Self::RenameTable { .. } => 5,
			Self::RenameColumn { .. }
			| Self::RenameIndex { .. }
			| Self::RenameConstraint { .. } => 6,
			Self::CreateTable(_) | Self::Repartition { .. } => 7,
			Self::CreateSequence(_) => 8,
			Self::CreateFunction(_) | Self::ReplaceFunction { .. } => 9,
			// A generated column goes before the columns it reads change type,
			// which Postgres refuses while something is computed from them.
			Self::AddColumn { .. } => 10,

			Self::DropColumn { column, .. } if column.generated.is_some() => 10,
			Self::AlterColumn { .. } => 11,
			Self::DropColumn { .. } => 12,
			Self::SetPrimaryKey { .. } => 13,
			Self::CreateIndex { .. } => 14,
			Self::AddCheck { .. } => 15,
			Self::AddForeignKey { .. } => 16,
			Self::CreateSearch { .. }
			| Self::CreateRowSecurity { .. }
			| Self::CreateGeoIndex { .. }
			| Self::CreateVectorIndex { .. }
			| Self::CreateVectorSource { .. }
			| Self::RegisterGeoPackage { .. } => 17,
			Self::SetTableComment { .. } | Self::SetColumnComment { .. } => 18,
			Self::DropTable(_) => 19,
			Self::DropFunction(_) => 20,
			Self::DropSequence(_) => 21,
			Self::CreateView(_) => 22,
			Self::CreateTrigger(_) => 23,
			Self::CreatePublication { .. } | Self::AlterPublication { .. } => 24,
		}
	}
}

/// The column, and what about it changed, one clause per change.
fn altered_column(
	f: &mut fmt::Formatter<'_>,
	table: &TableName,
	from: &Column,
	to: &Column,
) -> fmt::Result {
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

impl fmt::Display for Step {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::CreateTable(table) => {
				let count = table.columns.len();
				let noun = if count == 1 { "column" } else { "columns" };
				write!(f, "+ table {} ({count} {noun})", table.name)
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
			Self::AlterColumn { table, from, to } => altered_column(f, table, from, to),
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
			Self::CreateSearch { table, search } => {
				let columns: Vec<String> = search
					.columns
					.iter()
					.map(|column| format!("{} {}", column.name, column.weight))
					.collect();
				write!(
					f,
					"+ search index {table} ({}; {})",
					columns.join(", "),
					search.language
				)
			}
			Self::DropSearch { table, .. } => write!(f, "- search index {table}"),
			Self::CreateRowSecurity { table, security } => {
				let commands: Vec<&str> = security
					.policies
					.iter()
					.map(|policy| policy.command.keyword())
					.collect();
				write!(
					f,
					"+ row-level security on {table} ({})",
					commands.join(", ")
				)
			}
			Self::DropRowSecurity { table, .. } => {
				write!(f, "- row-level security on {table}")
			}
			Self::CreateGeoIndex { table, index } => {
				write!(f, "+ spatial index {table}({})", index.column)
			}
			Self::DropGeoIndex { table, index } => {
				write!(f, "- spatial index {table}({})", index.column)
			}
			Self::CreateVectorIndex { table, vector } => {
				write!(f, "+ vector index {table}({})", vector.column)?;

				if let Some(distance) = vector.distance {
					write!(f, " by {distance}")?;
				}

				Ok(())
			}
			Self::DropVectorIndex { table, vector } => {
				write!(f, "- vector index {table}({})", vector.column)
			}
			Self::CreateVectorSource { table, vector } => {
				write!(
					f,
					"+ vector sources {table}.{} ({})",
					vector.column,
					vector.sources.join(", ")
				)
			}
			Self::DropVectorSource { table, vector } => {
				write!(f, "- vector sources {table}.{}", vector.column)
			}
			Self::RegisterGeoPackage { table, layer } => {
				write!(f, "+ GeoPackage layer {table}({})", layer.column)
			}
			Self::UnregisterGeoPackage { table, layer } => {
				write!(f, "- GeoPackage layer {table}({})", layer.column)
			}
			Self::CreateView(view) => write!(f, "+ {} {}", view_kind(view), view.name),
			Self::DropView(view) => write!(f, "- {} {}", view_kind(view), view.name),
			Self::CreateSequence(sequence) => write!(f, "+ sequence {}", sequence.name),
			Self::DropSequence(sequence) => write!(f, "- sequence {}", sequence.name),
			Self::CreateFunction(function) => write!(f, "+ function {}", function.name),
			Self::ReplaceFunction { from, to } => {
				write!(
					f,
					"~ function {}: body {} → {}",
					to.name,
					short_checksum(&from.checksum),
					short_checksum(&to.checksum)
				)
			}
			Self::DropFunction(function) => write!(f, "- function {}", function.name),
			Self::CreateTrigger(trigger) => write!(f, "+ {trigger}"),
			Self::DropTrigger(trigger) => write!(f, "- {trigger}"),
			Self::Repartition { to, .. } => {
				match &to.partition {
					Some(partition) => {
						write!(
							f,
							"~ table {}: partitioned by ranges of {}",
							to.name, partition.column
						)
					}
					None => write!(f, "~ table {}: no longer partitioned", to.name),
				}
			}
			Self::CreatePublication { publication, .. } => write!(f, "+ {publication}"),
			Self::AlterPublication { from, to, .. } => altered_publication(f, from, to),
			Self::DropPublication { publication, .. } => {
				write!(f, "- publication {}", publication.name)
			}
		}
	}
}

/// The publication, and the tables it gained, lost, or describes anew.
fn altered_publication(
	f: &mut fmt::Formatter<'_>,
	from: &Publication,
	to: &Publication,
) -> fmt::Result {
	write!(f, "~ publication {}:", to.name)?;

	for table in &to.tables {
		match from.table(&table.table) {
			None => write!(f, " +{}", table.table)?,
			Some(old) if old != table => write!(f, " ~{}", table.table)?,
			Some(_) => {}
		}
	}

	for table in &from.tables {
		if to.table(&table.table).is_none() {
			write!(f, " -{}", table.table)?;
		}
	}

	Ok(())
}

/// A checksum's first eight digits, enough to tell two apart in a summary.
fn short_checksum(checksum: &str) -> &str {
	checksum.get(..8).unwrap_or(checksum)
}

fn view_kind(view: &View) -> &'static str {
	if view.materialized {
		"materialized view"
	} else {
		"view"
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
	/// A generated column's expression changed, or a column started or
	/// stopped being generated. Neither database changes that in place
	/// everywhere, so the migration would have to drop the column and its
	/// indexes.
	#[error(
		"{table}.{column} changes how it's generated, or whether it is; add a new column under \
		 another name and drop this one, so the migration rebuilds its indexes too"
	)]
	GeneratedChanged {
		/// The table.
		table: TableName,
		/// The column.
		column: String,
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

	// A table that becomes partitioned, or stops being, is rebuilt as the
	// target says, so it has no other steps.
	let (repartitioned, matched_tables): (Vec<_>, Vec<_>) = matched_tables
		.iter()
		.partition(|(pair, _)| pair.previous.partition != pair.target.partition);

	for (pair, columns) in &repartitioned {
		steps.push(repartition(pair, columns));
	}

	for (pair, columns) in &matched_tables {
		diff_table(
			target.dialect,
			pair,
			columns,
			&translate,
			&retyped,
			&rekeyed,
			&mut steps,
		)?;
	}

	for (pair, columns) in &matched_tables {
		diff_search(pair, columns, &rekeyed, &mut steps);
		diff_geo_indexes(pair, columns, &rekeyed, &mut steps);
		diff_vectors(target.dialect, pair, columns, &rekeyed, &mut steps);
		diff_geopackage(previous, target, pair, columns, &mut steps);
	}

	for table in pairs.added {
		create_table_steps(target, table, &mut steps);
	}

	for table in pairs.removed {
		drop_table_steps(previous, table, &mut steps);
	}

	diff_row_security(previous, target, &table_renames, &mut steps);
	diff_views(previous, target, &mut steps);
	diff_routines(previous, target, &mut steps);
	diff_publications(previous, target, &table_renames, &mut steps);

	// A stable sort keeps each phase in table and declaration order.
	steps.sort_by_key(Step::phase);

	Ok(Plan {
		dialect: target.dialect,
		steps,
	})
}

/// The step that rebuilds a table whose partitioning changes, copying the
/// columns it keeps.
fn repartition(pair: &TablePair<'_>, columns: &ColumnMatches<'_>) -> Step {
	let mut from = pair.previous.clone();
	from.name = pair.target.name.clone();

	Step::Repartition {
		from,
		to: pair.target.clone(),
		columns: columns
			.matched
			.iter()
			.filter(|(_, to)| to.generated.is_none())
			.map(|(from, to)| (from.name.clone(), to.name.clone()))
			.collect(),
	}
}

/// Creates, changes, and drops publications. A table renamed in the same
/// plan stays in its publication, which follows the table rather than its
/// name.
fn diff_publications(
	previous: &Schema,
	target: &Schema,
	table_renames: &HashMap<&TableName, &TableName>,
	steps: &mut Vec<Step>,
) {
	for publication in &previous.publications {
		if target.publication(&publication.name).is_none() {
			steps.push(Step::DropPublication {
				name: TableName::new(&publication.name),
				publication: publication.clone(),
			});
		}
	}

	for publication in &target.publications {
		let Some(old) = previous.publication(&publication.name) else {
			steps.push(Step::CreatePublication {
				name: TableName::new(&publication.name),
				publication: publication.clone(),
			});

			continue;
		};

		let renamed = Publication::new(
			old.name.clone(),
			old.tables
				.iter()
				.map(|table| {
					let mut table = table.clone();

					if let Some(name) = table_renames.get(&table.table) {
						table.table = (*name).clone();
					}

					table
				})
				.collect(),
		);

		if renamed != *publication {
			steps.push(Step::AlterPublication {
				name: TableName::new(&publication.name),
				from: renamed,
				to: publication.clone(),
			});
		}
	}
}

/// The steps that create `table`: its search, spatial, and vector indexes,
/// and its GeoPackage layer.
fn create_table_steps(target: &Schema, table: &Table, steps: &mut Vec<Step>) {
	steps.push(Step::CreateTable(table.clone()));

	for index in &table.geo_indexes {
		steps.push(Step::CreateGeoIndex {
			table: table.name.clone(),
			index: index.clone(),
		});
	}

	for vector in &table.vectors {
		steps.extend(create_vector(target.dialect, &table.name, vector));
	}

	if let Some(layer) = target.geopackage_layer(table) {
		steps.push(Step::RegisterGeoPackage {
			table: table.name.clone(),
			layer,
		});
	}

	if let Some(search) = &table.search {
		steps.push(Step::CreateSearch {
			table: table.name.clone(),
			search: search.clone(),
		});
	}
}

/// The steps that drop `table`, in the order its indexes and layer go.
fn drop_table_steps(previous: &Schema, table: &Table, steps: &mut Vec<Step>) {
	if let Some(layer) = previous.geopackage_layer(table) {
		steps.push(Step::UnregisterGeoPackage {
			table: table.name.clone(),
			layer,
		});
	}

	for index in &table.geo_indexes {
		steps.push(Step::DropGeoIndex {
			table: table.name.clone(),
			index: index.clone(),
		});
	}

	for vector in &table.vectors {
		steps.extend(drop_vector(previous.dialect, &table.name, vector));
	}

	if let Some(search) = &table.search {
		steps.push(Step::DropSearch {
			table: table.name.clone(),
			search: search.clone(),
		});
	}

	steps.push(Step::DropTable(table.clone()));
}

/// Drops the row-level security policies that changed, and creates them
/// again. A policy's SQL isn't parsed, and Postgres refuses to retype or
/// drop a column any policy reads, its counts of other tables included, so
/// when any table changes, every table's policies are recreated, as views
/// are. Policies go with a dropped table.
fn diff_row_security(
	previous: &Schema,
	target: &Schema,
	table_renames: &HashMap<&TableName, &TableName>,
	steps: &mut Vec<Step>,
) {
	let tables_change = steps.iter().any(|step| {
		!matches!(
			step,
			Step::CreateTable(_)
				| Step::CreateIndex { .. }
				| Step::SetTableComment { .. }
				| Step::SetColumnComment { .. }
				| Step::CreateSearch { .. }
				| Step::CreateGeoIndex { .. }
				| Step::CreateVectorIndex { .. }
				| Step::RegisterGeoPackage { .. }
		)
	});
	let renamed: HashMap<&TableName, &TableName> = table_renames
		.iter()
		.map(|(from, to)| (*to, *from))
		.collect();

	for table in &target.tables {
		let before = renamed
			.get(&table.name)
			.and_then(|name| previous.table(name))
			.or_else(|| previous.table(&table.name))
			.and_then(|previous| previous.row_security.as_ref());
		let after = table.row_security.as_ref();

		if !tables_change && before == after {
			continue;
		}

		if let Some(security) = before {
			steps.push(Step::DropRowSecurity {
				table: table.name.clone(),
				security: security.clone(),
			});
		}

		if let Some(security) = after {
			steps.push(Step::CreateRowSecurity {
				table: table.name.clone(),
				security: security.clone(),
			});
		}
	}
}

/// Drops the views that changed or disappeared and creates the ones that
/// changed or appeared. When a table changes in a way a view could notice,
/// every view is recreated: a view's SQL isn't parsed, so the tables it
/// reads aren't known, and both databases refuse to drop or retype a column
/// a view reads, and SQLite can't rebuild a table one reads.
///
/// A view that reads a recreated view is recreated too, since Postgres
/// refuses to drop a view another reads. Views are dropped with those that
/// read them first, and created after the views they read.
fn diff_views(previous: &Schema, target: &Schema, steps: &mut Vec<Step>) {
	let tables_change = steps.iter().any(|step| {
		!matches!(
			step,
			Step::CreateTable(_)
				| Step::CreateIndex { .. }
				| Step::SetTableComment { .. }
				| Step::SetColumnComment { .. }
				| Step::CreateRowSecurity { .. }
				| Step::DropRowSecurity { .. }
				| Step::CreateGeoIndex { .. }
				| Step::CreateVectorIndex { .. }
				| Step::RegisterGeoPackage { .. }
				| Step::CreateSequence(_)
				| Step::DropSequence(_)
				| Step::CreateFunction(_)
				| Step::ReplaceFunction { .. }
				| Step::DropFunction(_)
				| Step::CreateTrigger(_)
				| Step::DropTrigger(_)
		)
	});
	let unchanged = |view: &View| {
		!tables_change
			&& previous
				.view(&view.name)
				.zip(target.view(&view.name))
				.is_some_and(|(old, new)| old.same_on(new, target.dialect))
	};
	let mut recreated: HashSet<TableName> = previous
		.views
		.iter()
		.chain(&target.views)
		.filter(|view| !unchanged(view))
		.map(|view| view.name.clone())
		.collect();

	loop {
		let reading: Vec<TableName> = [previous, target]
			.into_iter()
			.flat_map(|schema| schema.views.iter().map(move |view| (schema.dialect, view)))
			.filter(|(dialect, view)| {
				!recreated.contains(&view.name)
					&& recreated
						.iter()
						.any(|name| *name != view.name && view.may_read(name, *dialect))
			})
			.map(|(_, view)| view.name.clone())
			.collect();

		if reading.is_empty() {
			break;
		}

		recreated.extend(reading);
	}

	let in_order = |schema: &'_ Schema| -> Vec<View> {
		schema
			.views_in_order()
			.unwrap_or_else(|_| schema.views.iter().collect())
			.into_iter()
			.filter(|view| recreated.contains(&view.name))
			.cloned()
			.collect()
	};

	// Each view goes before the views it reads, and by name otherwise.
	let mut dropping: Vec<&View> = previous
		.views
		.iter()
		.filter(|view| recreated.contains(&view.name))
		.collect();

	while !dropping.is_empty() {
		let next = dropping
			.iter()
			.position(|view| {
				!dropping.iter().any(|other| {
					other.name != view.name && other.may_read(&view.name, previous.dialect)
				})
			})
			// A valid schema has no cycle; if one slipped in, any order fails
			// the same way.
			.unwrap_or(0);
		steps.push(Step::DropView(dropping.remove(next).clone()));
	}

	for view in in_order(target) {
		steps.push(Step::CreateView(view));
	}
}

/// Recreates a table's search index when it changes, and whenever the
/// table changes under it: the table or a searched or key column is
/// renamed, retyped, or dropped, or the key changes. Postgres won't retype
/// or drop a column its generated `tsvector` reads, and SQLite's companion
/// tables are named after the table and keyed by its key columns.
fn diff_search(
	pair: &TablePair<'_>,
	columns: &ColumnMatches<'_>,
	rekeyed: &HashSet<TableName>,
	steps: &mut Vec<Step>,
) {
	let (previous, target) = (&pair.previous.search, &pair.target.search);
	let read = |column: &str, search: &Option<Search>, key: &[String]| {
		search.as_ref().is_some_and(|search| search.reads(column))
			|| key.iter().any(|name| name == column)
	};
	let column_changed = columns.matched.iter().any(|(from, to)| {
		(from.name != to.name || from.ty != to.ty)
			&& (read(&from.name, previous, &pair.previous.primary_key)
				|| read(&to.name, target, &pair.target.primary_key))
	}) || columns
		.removed
		.iter()
		.any(|column| read(&column.name, previous, &pair.previous.primary_key));
	let table_changed =
		pair.previous.name != pair.target.name || rekeyed.contains(&pair.target.name);

	if previous == target && !column_changed && !table_changed {
		return;
	}

	if let Some(search) = previous {
		steps.push(Step::DropSearch {
			table: pair.target.name.clone(),
			search: search.clone(),
		});
	}

	if let Some(search) = target {
		steps.push(Step::CreateSearch {
			table: pair.target.name.clone(),
			search: search.clone(),
		});
	}
}

/// Recreates each spatial index that changes, and each one the table
/// changes under: the table is renamed or rekeyed, or the indexed or a key
/// column is renamed, retyped, or dropped. SQLite's companion tables and
/// triggers are named after the table and column and keyed by the key, and
/// Postgres's index is named after them.
fn diff_geo_indexes(
	pair: &TablePair<'_>,
	columns: &ColumnMatches<'_>,
	rekeyed: &HashSet<TableName>,
	steps: &mut Vec<Step>,
) {
	let changed = |from: &Column, to: &Column| from.name != to.name || from.ty != to.ty;
	let key_moved = columns
		.matched
		.iter()
		.any(|(from, to)| changed(from, to) && pair.previous.primary_key.contains(&from.name))
		|| columns
			.removed
			.iter()
			.any(|column| pair.previous.primary_key.contains(&column.name));
	let table_changed =
		pair.previous.name != pair.target.name || rekeyed.contains(&pair.target.name) || key_moved;

	for index in &pair.previous.geo_indexes {
		let survives = pair
			.target
			.geo_indexes
			.iter()
			.any(|kept| kept.column == index.column);
		let disturbed = columns
			.matched
			.iter()
			.any(|(from, to)| from.name == index.column && changed(from, to))
			|| columns
				.removed
				.iter()
				.any(|column| column.name == index.column);

		if !survives || disturbed || table_changed {
			steps.push(Step::DropGeoIndex {
				table: pair.target.name.clone(),
				index: index.clone(),
			});
		}
	}

	for index in &pair.target.geo_indexes {
		let existed = pair
			.previous
			.geo_indexes
			.iter()
			.any(|old| old.column == index.column);
		let disturbed = columns
			.matched
			.iter()
			.any(|(from, to)| to.name == index.column && changed(from, to))
			|| columns
				.added
				.iter()
				.any(|column| column.name == index.column);

		if !existed || disturbed || table_changed {
			steps.push(Step::CreateGeoIndex {
				table: pair.target.name.clone(),
				index: index.clone(),
			});
		}
	}
}

/// The steps that index a new vector column and record its sources, as far
/// as `dialect` keeps either.
fn create_vector(dialect: Dialect, table: &TableName, vector: &VectorOptions) -> Vec<Step> {
	let mut steps = Vec::new();

	if vector.is_indexed(dialect) {
		steps.push(Step::CreateVectorIndex {
			table: table.clone(),
			vector: vector.clone(),
		});
	}

	if !vector.sources.is_empty() {
		steps.push(Step::CreateVectorSource {
			table: table.clone(),
			vector: vector.clone(),
		});
	}

	steps
}

/// The steps that undo [`create_vector`].
fn drop_vector(dialect: Dialect, table: &TableName, vector: &VectorOptions) -> Vec<Step> {
	let mut steps = Vec::new();

	if !vector.sources.is_empty() {
		steps.push(Step::DropVectorSource {
			table: table.clone(),
			vector: vector.clone(),
		});
	}

	if vector.is_indexed(dialect) {
		steps.push(Step::DropVectorIndex {
			table: table.clone(),
			vector: vector.clone(),
		});
	}

	steps
}

/// Recreates each vector column's index and source triggers that change,
/// and each one the table changes under, as [`diff_geo_indexes`] does: the
/// table is renamed or rekeyed, or a column they read (the vector, a source,
/// or a key column) is renamed, retyped, or dropped. Visibility alone
/// changes neither.
fn diff_vectors(
	dialect: Dialect,
	pair: &TablePair<'_>,
	columns: &ColumnMatches<'_>,
	rekeyed: &HashSet<TableName>,
	steps: &mut Vec<Step>,
) {
	// The previous names of the columns renamed, retyped, or dropped.
	let disturbed: HashSet<&str> = columns
		.matched
		.iter()
		.filter(|(from, to)| from.name != to.name || from.ty != to.ty)
		.map(|(from, _)| from.name.as_str())
		.chain(columns.removed.iter().map(|column| column.name.as_str()))
		.collect();
	let table_changed = pair.previous.name != pair.target.name
		|| rekeyed.contains(&pair.target.name)
		|| pair
			.previous
			.primary_key
			.iter()
			.any(|key| disturbed.contains(key.as_str()));
	let (kept_indexes, kept_sources) =
		drop_vectors(dialect, pair, &disturbed, table_changed, steps);

	create_vectors(dialect, pair, &kept_indexes, &kept_sources, steps);
}

/// Drops the vector indexes and sources that changed, and says which the
/// target keeps.
fn drop_vectors<'a>(
	dialect: Dialect,
	pair: &'a TablePair<'_>,
	disturbed: &HashSet<&str>,
	table_changed: bool,
	steps: &mut Vec<Step>,
) -> (HashSet<&'a str>, HashSet<&'a str>) {
	let mut kept_indexes = HashSet::new();
	let mut kept_sources = HashSet::new();

	for vector in &pair.previous.vectors {
		let kept = pair
			.target
			.vectors
			.iter()
			.find(|kept| kept.column == vector.column)
			.filter(|_| !table_changed && !disturbed.contains(vector.column.as_str()));

		if vector.is_indexed(dialect) {
			if kept
				.is_some_and(|kept| (kept.distance, kept.index) == (vector.distance, vector.index))
			{
				kept_indexes.insert(vector.column.as_str());
			} else {
				steps.push(Step::DropVectorIndex {
					table: pair.target.name.clone(),
					vector: vector.clone(),
				});
			}
		}

		if !vector.sources.is_empty() {
			let sources_kept = kept.is_some_and(|kept| kept.sources == vector.sources)
				&& !vector
					.sources
					.iter()
					.any(|source| disturbed.contains(source.as_str()));

			if sources_kept {
				kept_sources.insert(vector.column.as_str());
			} else {
				steps.push(Step::DropVectorSource {
					table: pair.target.name.clone(),
					vector: vector.clone(),
				});
			}
		}
	}

	(kept_indexes, kept_sources)
}

/// Creates the target's vector indexes and sources that aren't kept.
fn create_vectors(
	dialect: Dialect,
	pair: &TablePair<'_>,
	kept_indexes: &HashSet<&str>,
	kept_sources: &HashSet<&str>,
	steps: &mut Vec<Step>,
) {
	for vector in &pair.target.vectors {
		if vector.is_indexed(dialect) && !kept_indexes.contains(vector.column.as_str()) {
			steps.push(Step::CreateVectorIndex {
				table: pair.target.name.clone(),
				vector: vector.clone(),
			});
		}

		if !vector.sources.is_empty() && !kept_sources.contains(vector.column.as_str()) {
			steps.push(Step::CreateVectorSource {
				table: pair.target.name.clone(),
				vector: vector.clone(),
			});
		}
	}
}
/// Registers a table's GeoPackage layer again when it changes, or when the
/// table or its geometry column is renamed.
fn diff_geopackage(
	previous: &Schema,
	target: &Schema,
	pair: &TablePair<'_>,
	columns: &ColumnMatches<'_>,
	steps: &mut Vec<Step>,
) {
	let before = previous.geopackage_layer(pair.previous);
	let after = target.geopackage_layer(pair.target);
	let renamed = pair.previous.name != pair.target.name
		|| before
			.as_ref()
			.is_some_and(|layer| columns.renames.contains_key(&layer.column));

	if before == after && !renamed {
		return;
	}

	if let Some(layer) = before {
		steps.push(Step::UnregisterGeoPackage {
			table: pair.target.name.clone(),
			layer,
		});
	}

	if let Some(layer) = after {
		steps.push(Step::RegisterGeoPackage {
			table: pair.target.name.clone(),
			layer,
		});
	}
}

/// Creates, replaces, and drops sequences, functions, and triggers.
///
/// A trigger whose definition changed, or whose table was renamed, is
/// dropped and created again. On SQLite, so is every trigger on a table the
/// plan alters at all: a rebuild drops the table and its triggers with it,
/// and `ALTER TABLE` may rewrite a trigger's statements. Called once every
/// table step is in `steps`.
fn diff_routines(previous: &Schema, target: &Schema, steps: &mut Vec<Step>) {
	diff_sequences(previous, target, steps);
	diff_functions(previous, target, steps);
	diff_triggers(previous, target, steps);
}

/// Drops the sequences that went and creates the ones that came.
fn diff_sequences(previous: &Schema, target: &Schema, steps: &mut Vec<Step>) {
	for sequence in &previous.sequences {
		if !target
			.sequences
			.iter()
			.any(|kept| kept.name == sequence.name)
		{
			steps.push(Step::DropSequence(sequence.clone()));
		}
	}

	for sequence in &target.sequences {
		if !previous
			.sequences
			.iter()
			.any(|old| old.name == sequence.name)
		{
			steps.push(Step::CreateSequence(sequence.clone()));
		}
	}
}

/// Drops the functions that went, creates the ones that came, and replaces
/// the ones whose body changed.
fn diff_functions(previous: &Schema, target: &Schema, steps: &mut Vec<Step>) {
	for function in &previous.functions {
		if target.function(&function.name).is_none() {
			steps.push(Step::DropFunction(function.clone()));
		}
	}

	for function in &target.functions {
		match previous.function(&function.name) {
			None => steps.push(Step::CreateFunction(function.clone())),

			Some(old) if old != function => {
				steps.push(Step::ReplaceFunction {
					from: old.clone(),
					to: function.clone(),
				});
			}

			Some(_) => {}
		}
	}
}

/// Drops and creates the triggers a rebuilt table lost, and the ones that
/// changed.
fn diff_triggers(previous: &Schema, target: &Schema, steps: &mut Vec<Step>) {
	let altered: HashSet<TableName> = if target.dialect == Dialect::Sqlite {
		steps
			.iter()
			.filter(|step| {
				!matches!(
					step,
					Step::SetTableComment { .. }
						| Step::SetColumnComment { .. }
						| Step::CreateIndex { .. }
						| Step::DropIndex { .. }
						| Step::RenameIndex { .. }
						| Step::RenameConstraint { .. }
						| Step::CreateView(_)
						| Step::DropView(_)
				)
			})
			.map(|step| step.table().clone())
			.collect()
	} else {
		// A rebuilt table loses its triggers with the old one.
		steps
			.iter()
			.filter(|step| matches!(step, Step::Repartition { .. }))
			.map(|step| step.table().clone())
			.collect()
	};

	let kept = |trigger: &Trigger| {
		!altered.contains(&trigger.table)
			&& previous.triggers.contains(trigger)
			&& target.triggers.contains(trigger)
	};

	for trigger in previous.triggers.iter().filter(|trigger| !kept(trigger)) {
		// Dropping a table drops its triggers, but the table goes last, and
		// dropping the trigger first costs nothing.
		steps.push(Step::DropTrigger(trigger.clone()));
	}

	for trigger in target.triggers.iter().filter(|trigger| !kept(trigger)) {
		steps.push(Step::CreateTrigger(trigger.clone()));
	}
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
			| CheckRule::Regex { column, .. }
			| CheckRule::ValidGeometry { column } => *column = self.rename(column),
			CheckRule::Sql { .. } => {}
		}

		check
	}
}

fn diff_table(
	dialect: Dialect,
	pair: &TablePair<'_>,
	columns: &ColumnMatches<'_>,
	translate: &Translate<'_>,
	retyped: &HashSet<(TableName, String)>,
	rekeyed: &HashSet<TableName>,
	steps: &mut Vec<Step>,
) -> Result<(), DiffError> {
	let table = &pair.target.name;
	diff_columns(dialect, table, columns, steps)?;
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
	dialect: Dialect,
	table: &TableName,
	columns: &ColumnMatches<'_>,
	steps: &mut Vec<Step>,
) -> Result<(), DiffError> {
	for (from, to) in &columns.matched {
		if from.generated_sql(dialect) != to.generated_sql(dialect) {
			return Err(DiffError::GeneratedChanged {
				table: table.clone(),
				column: to.name.clone(),
			});
		}

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
		// The database computes a generated column for the rows already there.
		if !column.nullable && column.default.is_none() && column.generated.is_none() {
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

	// A generated column goes before the columns it reads, which the
	// database won't drop while something is computed from them.
	let mut removed = columns.removed.clone();
	removed.sort_by_key(|column| column.generated.is_none());

	for column in removed {
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
