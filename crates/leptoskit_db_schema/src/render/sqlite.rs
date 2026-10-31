//! SQLite: `STRICT` tables, and table rebuilds for every change `ALTER TABLE`
//! can't make.
//!
//! SQLite's `ALTER TABLE` can rename tables and columns, add a column with a
//! constant default, and drop an unconstrained column. Everything else (types,
//! nullability, keys, foreign keys, checks) follows SQLite's documented
//! procedure: create the new table, copy the rows, drop the old table, and
//! rename the new one into place. Rebuilds run with foreign key enforcement
//! off, and the migrator checks every foreign key before committing.
//!
//! Renames always run first as plain `ALTER TABLE` statements, even for tables
//! that are rebuilt afterwards, because only a rename updates the foreign keys
//! in other tables that point at the renamed table or column.

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;

use super::Migration;
use super::float_literal;
use super::names_before_renames;
use super::number_literal;
use super::postgres::predicate_sql;
use crate::Check;
use crate::CheckRule;
use crate::Column;
use crate::ColumnType;
use crate::DefaultValue;
use crate::Dialect;
use crate::ForeignKey;
use crate::Index;
use crate::Plan;
use crate::Schema;
use crate::Step;
use crate::Table;
use crate::TableName;
use crate::View;
use crate::quote_identifier as ident;
use crate::quote_literal as literal;

/// The format SQLite stores timestamps in: RFC 3339 in UTC with exactly three
/// fractional digits, so that text order is time order.
const NOW: &str = "(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))";

/// The prefix for the temporary table a rebuild copies rows into.
const REBUILD_PREFIX: &str = "_leptoskit_rebuild_";

pub(super) fn render(plan: &Plan, previous: &Schema, target: &Schema) -> Migration {
	let before_renames = names_before_renames(plan);
	let rebuilt = tables_to_rebuild(plan, previous, &before_renames);
	let mut statements = Vec::new();
	let mut notes = Vec::new();
	let mut rebuilds_written = false;
	let mut created_views = Vec::new();

	for step in &plan.steps {
		let table = step.table();

		// Rebuilds run after every rename, new table, and simple new column, so
		// their copies can read the new names.
		if !rebuilds_written && step_phase_reaches_rebuilds(step) {
			write_rebuilds(&rebuilt, plan, target, &mut statements, &mut notes);
			rebuilds_written = true;
		}

		if rebuilt.contains(table)
			&& !matches!(step, Step::RenameTable { .. } | Step::RenameColumn { .. })
		{
			continue;
		}

		match step {
			Step::DropView(view) => {
				let kind = if view.materialized { "TABLE" } else { "VIEW" };
				statements.push(format!("DROP {kind} {}", ident(&view.name.flat())));
			}
			Step::CreateView(view) => created_views.extend(create_view(view)),
			Step::CreateTable(table) => {
				statements.push(create_table(table, &mut notes));
				statements.extend(
					table
						.indexes
						.iter()
						.map(|index| create_index(&table.name, index)),
				);
			}
			Step::RenameTable { from, to } => {
				statements.push(format!(
					"ALTER TABLE {} RENAME TO {}",
					ident(&from.flat()),
					ident(&to.flat())
				));
			}
			Step::RenameColumn { from, to, .. } => {
				statements.push(format!(
					"ALTER TABLE {} RENAME COLUMN {} TO {}",
					ident(&table.flat()),
					ident(from),
					ident(to)
				));
			}
			Step::AddColumn { column, .. } => {
				statements.push(format!(
					"ALTER TABLE {} ADD COLUMN {}",
					ident(&table.flat()),
					column_definition(column)
				));
			}
			Step::DropColumn { column, .. } => {
				statements.push(format!(
					"ALTER TABLE {} DROP COLUMN {}",
					ident(&table.flat()),
					ident(&column.name)
				));
			}
			Step::CreateIndex { index, .. } => statements.push(create_index(table, index)),
			Step::DropIndex { index, .. } => {
				statements.push(format!("DROP INDEX {}", ident(&index.name)));
			}
			// SQLite can't rename an index, so it is rebuilt under its new
			// name.
			Step::RenameIndex { from, index, .. } => {
				statements.push(format!("DROP INDEX {}", ident(from)));
				statements.push(create_index(table, index));
			}
			// Dropped tables go last, in dependency order. SQLite keeps no
			// comments; schema.json records them. Constraint names are only
			// labels in a table's SQL that nothing refers to, and the next
			// rebuild writes the new name.
			Step::DropTable(_)
			| Step::SetTableComment { .. }
			| Step::SetColumnComment { .. }
			| Step::RenameConstraint { .. } => {}
			Step::AlterColumn { .. }
			| Step::SetPrimaryKey { .. }
			| Step::AddForeignKey { .. }
			| Step::DropForeignKey { .. }
			| Step::AddCheck { .. }
			| Step::DropCheck { .. } => {
				unreachable!("`tables_to_rebuild` rebuilds every table with this step")
			}
		}
	}

	if !rebuilds_written {
		write_rebuilds(&rebuilt, plan, target, &mut statements, &mut notes);
	}

	let (drops, drops_a_cycle) = drop_tables(plan);
	statements.extend(drops);
	statements.append(&mut created_views);

	Migration {
		dialect: Dialect::Sqlite,
		statements,
		foreign_keys_off: !rebuilt.is_empty() || drops_a_cycle,
		notes,
	}
}

/// Whether the step comes after `AddColumn` in plan order, where rebuilds go.
/// Dropping a generated column shares `AddColumn`'s place, and always
/// rebuilds its table, so it waits there too.
fn step_phase_reaches_rebuilds(step: &Step) -> bool {
	if matches!(step, Step::DropColumn { column, .. } if column.generated.is_some()) {
		return false;
	}

	!matches!(
		step,
		Step::DropView(_)
			| Step::DropForeignKey { .. }
			| Step::DropIndex { .. }
			| Step::DropCheck { .. }
			| Step::RenameTable { .. }
			| Step::RenameColumn { .. }
			| Step::CreateTable(_)
			| Step::AddColumn { .. }
	)
}

/// The tables, by their new names, that `ALTER TABLE` can't change in place.
fn tables_to_rebuild(
	plan: &Plan,
	previous: &Schema,
	before_renames: &HashMap<TableName, TableName>,
) -> BTreeSet<TableName> {
	let mut rebuilt = BTreeSet::new();

	for step in &plan.steps {
		let table = step.table();
		let needs_rebuild = match step {
			Step::AlterColumn { .. }
			| Step::SetPrimaryKey { .. }
			| Step::AddForeignKey { .. }
			| Step::DropForeignKey { .. }
			| Step::AddCheck { .. }
			| Step::DropCheck { .. } => true,
			// `ADD COLUMN` takes only constant defaults, and no stored
			// generated column.
			Step::AddColumn { column, .. } => {
				column.generated.is_some()
					|| matches!(
						column.default,
						Some(DefaultValue::Now | DefaultValue::Sql(_))
					)
			}
			// `DROP COLUMN` refuses columns in a key, foreign key, or
			// table-level check.
			Step::DropColumn { column, .. } => {
				let old_name = before_renames.get(table).unwrap_or(table);
				previous
					.table(old_name)
					.is_some_and(|table| is_constrained(table, &column.name))
			}
			_ => false,
		};

		if needs_rebuild {
			rebuilt.insert(table.clone());
		}
	}

	rebuilt
}

/// Whether `DROP COLUMN` would refuse `column`: SQLite won't drop a column
/// in a key, foreign key, or table-level check, nor one a generated column
/// reads, which isn't known without parsing its SQL, so any table with a
/// generated column is rebuilt instead.
fn is_constrained(table: &Table, column: &str) -> bool {
	table.columns.iter().any(|other| other.generated.is_some())
		|| table.primary_key.iter().any(|key| key == column)
		|| table
			.foreign_keys
			.iter()
			.any(|key| key.columns.iter().any(|name| name == column))
		|| table
			.checks
			.iter()
			.any(|check| check.rule.column().is_none_or(|name| name == column))
}

fn write_rebuilds(
	rebuilt: &BTreeSet<TableName>,
	plan: &Plan,
	target: &Schema,
	statements: &mut Vec<String>,
	notes: &mut Vec<String>,
) {
	for name in rebuilt {
		let table = target
			.table(name)
			.expect("a rebuilt table is in the target schema");
		rebuild(table, plan, statements, notes);
	}
}

/// Rebuilds `table` from its new definition. Renames have already run, so
/// every surviving column has its new name in the old table.
fn rebuild(table: &Table, plan: &Plan, statements: &mut Vec<String>, notes: &mut Vec<String>) {
	let mut added = HashSet::new();
	let mut altered = HashMap::new();

	for step in plan.steps.iter().filter(|step| step.table() == &table.name) {
		match step {
			Step::AddColumn { column, .. } => {
				added.insert(column.name.as_str());
			}
			Step::AlterColumn { from, to, .. } => {
				altered.insert(to.name.as_str(), (from, to));
			}
			_ => {}
		}
	}

	let name = table.name.flat();
	let temporary = format!("{REBUILD_PREFIX}{name}");
	statements.push(create_table_named(&temporary, table, notes));

	// The database computes generated columns as it copies the rows.
	let copied: Vec<&Column> = table
		.columns
		.iter()
		.filter(|column| column.generated.is_none())
		.collect();
	let columns: Vec<String> = copied.iter().map(|column| ident(&column.name)).collect();
	let values: Vec<String> = copied
		.iter()
		.map(|column| {
			if added.contains(column.name.as_str()) {
				return column
					.default
					.as_ref()
					.map_or_else(|| "NULL".to_string(), default_expression);
			}

			let mut value = ident(&column.name);

			if let Some((from, to)) = altered.get(column.name.as_str()) {
				if storage_type(&from.ty) != storage_type(&to.ty) {
					value = format!("CAST({value} AS {})", storage_type(&to.ty));
				}

				if from.nullable
					&& !to.nullable
					&& let Some(default) = &to.default
				{
					value = format!("COALESCE({value}, {})", default_expression(default));
				}
			}

			value
		})
		.collect();

	statements.push(format!(
		"INSERT INTO {} ({}) SELECT {} FROM {}",
		ident(&temporary),
		columns.join(", "),
		values.join(", "),
		ident(&name)
	));
	statements.push(format!("DROP TABLE {}", ident(&name)));
	statements.push(format!(
		"ALTER TABLE {} RENAME TO {}",
		ident(&temporary),
		ident(&name)
	));
	statements.extend(
		table
			.indexes
			.iter()
			.map(|index| create_index(&table.name, index)),
	);
}

/// Drops tables so that referencing tables go before the tables they
/// reference: a parent's implicit `DELETE` would otherwise trip the child's
/// foreign key. Tables in a reference cycle have no such order; they are
/// dropped in plan order, and the second value is `true` so that the
/// migration runs with foreign keys off.
fn drop_tables(plan: &Plan) -> (Vec<String>, bool) {
	let mut remaining: Vec<&Table> = plan
		.steps
		.iter()
		.filter_map(|step| {
			match step {
				Step::DropTable(table) => Some(table),
				_ => None,
			}
		})
		.collect();
	let mut statements = Vec::new();
	let mut cycle = false;

	while !remaining.is_empty() {
		let unreferenced = remaining.iter().position(|table| {
			!remaining.iter().any(|other| {
				other.name != table.name
					&& other
						.foreign_keys
						.iter()
						.any(|key| key.references == table.name)
			})
		});
		cycle |= unreferenced.is_none();
		let table = remaining.remove(unreferenced.unwrap_or(0));
		statements.push(format!("DROP TABLE {}", ident(&table.name.flat())));
	}

	(statements, cycle)
}

/// A plain view: `CREATE VIEW name (columns) AS query`. SQLite has no
/// materialized views, so a materialized one is a table of its columns,
/// filled from its query now and on every refresh.
fn create_view(view: &View) -> Vec<String> {
	let name = ident(&view.name.flat());
	let names: Vec<String> = view
		.columns
		.iter()
		.map(|column| column.name.clone())
		.collect();
	let query = view.sql.sqlite.trim();

	if !view.materialized {
		return vec![format!(
			"CREATE VIEW {name} ({}) AS\n{query}",
			columns(&names)
		)];
	}

	let definitions: Vec<String> = view.columns.iter().map(column_definition).collect();
	let mut statements = vec![
		format!(
			"CREATE TABLE {name} (\n\t{}\n) STRICT",
			definitions.join(",\n\t")
		),
		format!("INSERT INTO {name} ({})\n{query}", columns(&names)),
	];
	statements.extend(
		view.indexes
			.iter()
			.map(|index| create_index(&view.name, index)),
	);
	statements
}

fn create_table(table: &Table, notes: &mut Vec<String>) -> String {
	create_table_named(&table.name.flat(), table, notes)
}

fn create_table_named(name: &str, table: &Table, notes: &mut Vec<String>) -> String {
	let mut lines: Vec<String> = table.columns.iter().map(column_definition).collect();
	lines.push(format!(
		"CONSTRAINT {} PRIMARY KEY ({})",
		ident(&table.primary_key_name()),
		columns(&table.primary_key)
	));

	for foreign_key in &table.foreign_keys {
		lines.push(foreign_key_constraint(foreign_key));
	}

	for check in &table.checks {
		match check_constraint(check) {
			Some(constraint) => lines.push(constraint),
			None => {
				notes.push(format!(
					"`{}.{}` isn't checked by SQLite, which has no built-in regular expressions; \
					 the app validates it",
					table.name,
					check.rule.column().unwrap_or_default()
				));
			}
		}
	}

	format!(
		"CREATE TABLE {} (\n\t{}\n) STRICT",
		ident(name),
		lines.join(",\n\t")
	)
}

fn columns(columns: &[String]) -> String {
	columns
		.iter()
		.map(|column| ident(column))
		.collect::<Vec<_>>()
		.join(", ")
}

/// The `STRICT` storage type for a column type.
fn storage_type(ty: &ColumnType) -> &'static str {
	match ty {
		ColumnType::Bool | ColumnType::Int16 | ColumnType::Int32 | ColumnType::Int64 => "INTEGER",
		ColumnType::Float32 | ColumnType::Float64 => "REAL",
		ColumnType::Decimal { .. }
		| ColumnType::Text { .. }
		| ColumnType::Timestamp
		| ColumnType::Date
		| ColumnType::Json => "TEXT",
		ColumnType::Bytes | ColumnType::Uuid => "BLOB",
		ColumnType::Array { .. } => unreachable!("validation rejects arrays on SQLite"),
	}
}

/// The check SQLite needs to hold a column to its type, beyond the storage
/// class `STRICT` enforces.
fn type_check(column: &Column) -> Option<String> {
	let name = ident(&column.name);

	match &column.ty {
		ColumnType::Bool => Some(format!("{name} IN (0, 1)")),
		ColumnType::Int16 => Some(format!("{name} BETWEEN -32768 AND 32767")),
		ColumnType::Int32 => Some(format!("{name} BETWEEN -2147483648 AND 2147483647")),
		ColumnType::Uuid => Some(format!("length({name}) = 16")),
		ColumnType::Text { max_len: Some(max) } => Some(format!("length({name}) <= {max}")),
		ColumnType::Json => Some(format!("json_valid({name})")),
		_ => None,
	}
}

fn default_expression(default: &DefaultValue) -> String {
	match default {
		DefaultValue::Bool(value) => i32::from(*value).to_string(),
		DefaultValue::Integer(value) => value.to_string(),
		DefaultValue::Float(value) => float_literal(*value),
		DefaultValue::Text(value) => literal(value),
		DefaultValue::Now => NOW.to_string(),
		DefaultValue::Sql(sql) => format!("({sql})"),
	}
}

fn column_definition(column: &Column) -> String {
	let mut definition = format!("{} {}", ident(&column.name), storage_type(&column.ty));

	if !column.nullable {
		definition.push_str(" NOT NULL");
	}

	if let Some(default) = &column.default {
		definition.push_str(" DEFAULT ");
		definition.push_str(&default_expression(default));
	}

	if let Some(sql) = column.generated_sql(Dialect::Sqlite) {
		definition.push_str(" GENERATED ALWAYS AS (");
		definition.push_str(sql);
		definition.push_str(") STORED");
	}

	if let Some(check) = type_check(column) {
		definition.push_str(" CHECK (");
		definition.push_str(&check);
		definition.push(')');
	}

	definition
}

fn create_index(table: &TableName, index: &Index) -> String {
	let unique = if index.unique { "UNIQUE " } else { "" };
	let mut sql = format!(
		"CREATE {unique}INDEX {} ON {} ({})",
		ident(&index.name),
		ident(&table.flat()),
		columns(&index.columns)
	);

	if let Some(predicate) = &index.predicate {
		sql.push_str(" WHERE ");
		sql.push_str(&predicate_sql(predicate));
	}

	sql
}

fn foreign_key_constraint(foreign_key: &ForeignKey) -> String {
	let mut sql = format!(
		"CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {}",
		ident(&foreign_key.name),
		columns(&foreign_key.columns),
		ident(&foreign_key.references.flat()),
		columns(&foreign_key.referenced_columns),
		foreign_key.on_delete.sql()
	);

	if foreign_key.deferred {
		sql.push_str(" DEFERRABLE INITIALLY DEFERRED");
	}

	sql
}

/// A check constraint, or `None` for a regular expression, which SQLite can't
/// evaluate.
fn check_constraint(check: &Check) -> Option<String> {
	let expression = match &check.rule {
		CheckRule::Range { column, min, max } => {
			let column = ident(column);
			let mut parts = Vec::new();

			if let Some(min) = min {
				parts.push(format!("{column} >= {}", number_literal(*min)));
			}

			if let Some(max) = max {
				parts.push(format!("{column} <= {}", number_literal(*max)));
			}

			if parts.is_empty() {
				"1".into()
			} else {
				parts.join(" AND ")
			}
		}
		CheckRule::Length { column, min, max } => {
			let length = format!("length({})", ident(column));
			let mut parts = Vec::new();

			if let Some(min) = min {
				parts.push(format!("{length} >= {min}"));
			}

			if let Some(max) = max {
				parts.push(format!("{length} <= {max}"));
			}

			if parts.is_empty() {
				"1".into()
			} else {
				parts.join(" AND ")
			}
		}
		CheckRule::OneOf { column, values } => {
			let values: Vec<String> = values.iter().map(|value| literal(value)).collect();
			format!("{} IN ({})", ident(column), values.join(", "))
		}
		CheckRule::Regex { .. } => return None,
		CheckRule::Sql { sql } => sql.clone(),
	};

	Some(format!(
		"CONSTRAINT {} CHECK ({expression})",
		ident(&check.name)
	))
}
