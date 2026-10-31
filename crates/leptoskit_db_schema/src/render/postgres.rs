//! PostgreSQL: every step is an `ALTER`, in plan order, except that foreign
//! keys on new tables are added once every table exists.

use std::collections::BTreeSet;
use std::collections::HashMap;

use super::Migration;
use super::float_literal;
use super::names_before_renames;
use super::number_literal;
use crate::Check;
use crate::CheckRule;
use crate::Column;
use crate::ColumnType;
use crate::DefaultValue;
use crate::Dialect;
use crate::ForeignKey;
use crate::Index;
use crate::Plan;
use crate::Predicate;
use crate::Schema;
use crate::Step;
use crate::Table;
use crate::TableName;
use crate::View;
use crate::quote_identifier as ident;
use crate::quote_literal as literal;

pub(super) fn render(plan: &Plan, previous: &Schema, target: &Schema) -> Migration {
	let mut writer = Writer {
		statements: Vec::new(),
		before_renames: names_before_renames(plan),
	};

	let namespaces = |schema: &'_ Schema| -> BTreeSet<String> {
		schema
			.tables
			.iter()
			.map(|table| &table.name)
			.chain(schema.views.iter().map(|view| &view.name))
			.filter_map(|name| name.namespace.clone())
			.collect()
	};
	let existing = namespaces(previous);
	let wanted = namespaces(target);

	for namespace in wanted.difference(&existing) {
		writer
			.statements
			.push(format!("CREATE SCHEMA IF NOT EXISTS {}", ident(namespace)));
	}

	let mut pending_foreign_keys = Vec::new();
	let mut dropped_tables = Vec::new();
	let mut created_views = Vec::new();

	for step in &plan.steps {
		if matches!(
			step,
			Step::AddForeignKey { .. }
				| Step::SetTableComment { .. }
				| Step::SetColumnComment { .. }
				| Step::DropTable(_)
		) {
			writer.statements.append(&mut pending_foreign_keys);
		}

		match step {
			Step::CreateTable(table) => {
				writer.create_table(table);

				for foreign_key in &table.foreign_keys {
					pending_foreign_keys.push(add_foreign_key(&table.name, foreign_key));
				}
			}
			Step::DropTable(table) => dropped_tables.push(qualified(&table.name)),
			Step::CreateView(view) => created_views.extend(create_view(view)),
			Step::DropView(view) => {
				writer.statements.push(format!(
					"DROP {} {}",
					view_keyword(view),
					qualified(&view.name)
				));
			}
			step => writer.step(step),
		}
	}

	writer.statements.append(&mut pending_foreign_keys);

	// One statement, so Postgres resolves dependencies among them.
	if !dropped_tables.is_empty() {
		writer
			.statements
			.push(format!("DROP TABLE {}", dropped_tables.join(", ")));
	}

	writer.statements.append(&mut created_views);

	Migration {
		dialect: Dialect::Postgres,
		statements: writer.statements,
		foreign_keys_off: false,
		notes: Vec::new(),
	}
}

struct Writer {
	statements: Vec<String>,
	before_renames: HashMap<TableName, TableName>,
}

impl Writer {
	/// The table's name when `step` runs: drops run before renames.
	fn name_for<'a>(&'a self, step: &Step, table: &'a TableName) -> &'a TableName {
		match step {
			Step::DropForeignKey { .. } | Step::DropIndex { .. } | Step::DropCheck { .. } => {
				self.before_renames.get(table).unwrap_or(table)
			}
			_ => table,
		}
	}

	fn alter(&mut self, table: &TableName, action: impl AsRef<str>) {
		self.statements.push(format!(
			"ALTER TABLE {} {}",
			qualified(table),
			action.as_ref()
		));
	}

	fn create_table(&mut self, table: &Table) {
		let mut lines: Vec<String> = table.columns.iter().map(column_definition).collect();
		lines.push(format!(
			"CONSTRAINT {} PRIMARY KEY ({})",
			ident(&table.primary_key_name()),
			columns(&table.primary_key)
		));

		for check in &table.checks {
			lines.push(check_constraint(check));
		}

		self.statements.push(format!(
			"CREATE TABLE {} (\n\t{}\n)",
			qualified(&table.name),
			lines.join(",\n\t")
		));

		for index in &table.indexes {
			self.statements.push(create_index(&table.name, index));
		}

		if let Some(comment) = &table.comment {
			self.statements.push(format!(
				"COMMENT ON TABLE {} IS {}",
				qualified(&table.name),
				literal(comment)
			));
		}

		for column in &table.columns {
			if let Some(comment) = &column.comment {
				self.statements
					.push(column_comment(&table.name, &column.name, Some(comment)));
			}
		}
	}

	fn step(&mut self, step: &Step) {
		let table = self.name_for(step, step.table()).clone();

		match step {
			Step::CreateTable(_) | Step::DropTable(_) | Step::CreateView(_) | Step::DropView(_) => {
				unreachable!("handled by `render`")
			}
			Step::RenameTable { from, to } => self.rename_table(from, to),
			Step::AddColumn { column, .. } => {
				self.alter(&table, format!("ADD COLUMN {}", column_definition(column)));
			}
			Step::DropColumn { column, .. } => {
				self.alter(&table, format!("DROP COLUMN {}", ident(&column.name)));
			}
			Step::RenameColumn { from, to, .. } => {
				self.alter(
					&table,
					format!("RENAME COLUMN {} TO {}", ident(from), ident(to)),
				);
			}
			Step::AlterColumn { from, to, .. } => self.alter_column(&table, from, to),
			Step::SetPrimaryKey { to, .. } => {
				let name = ident(&crate::constraint_name(&table, &[], "pkey"));
				self.alter(&table, format!("DROP CONSTRAINT {name}"));
				self.alter(
					&table,
					format!("ADD CONSTRAINT {name} PRIMARY KEY ({})", columns(to)),
				);
			}
			Step::CreateIndex { index, .. } => self.statements.push(create_index(&table, index)),
			Step::DropIndex { index, .. } => {
				let name = match &table.namespace {
					Some(namespace) => format!("{}.{}", ident(namespace), ident(&index.name)),
					None => ident(&index.name),
				};
				self.statements.push(format!("DROP INDEX {name}"));
			}
			Step::RenameIndex { from, index, .. } => {
				let from = match &table.namespace {
					Some(namespace) => format!("{}.{}", ident(namespace), ident(from)),
					None => ident(from),
				};
				self.statements.push(format!(
					"ALTER INDEX {from} RENAME TO {}",
					ident(&index.name)
				));
			}
			Step::RenameConstraint { from, to, .. } => {
				self.alter(
					&table,
					format!("RENAME CONSTRAINT {} TO {}", ident(from), ident(to)),
				);
			}
			Step::AddForeignKey { foreign_key, .. } => {
				self.statements.push(add_foreign_key(&table, foreign_key));
			}
			Step::DropForeignKey { foreign_key, .. } => {
				self.alter(
					&table,
					format!("DROP CONSTRAINT {}", ident(&foreign_key.name)),
				);
			}
			Step::AddCheck { check, .. } => {
				self.alter(&table, format!("ADD {}", check_constraint(check)));
			}
			Step::DropCheck { check, .. } => {
				self.alter(&table, format!("DROP CONSTRAINT {}", ident(&check.name)));
			}
			Step::SetTableComment { comment, .. } => {
				let comment = comment
					.as_deref()
					.map_or_else(|| "NULL".to_string(), literal);
				self.statements.push(format!(
					"COMMENT ON TABLE {} IS {comment}",
					qualified(&table)
				));
			}
			Step::SetColumnComment {
				column, comment, ..
			} => {
				self.statements
					.push(column_comment(&table, column, comment.as_deref()));
			}
		}
	}

	fn rename_table(&mut self, from: &TableName, to: &TableName) {
		let mut current = from.clone();

		if from.namespace != to.namespace {
			let namespace = to.namespace.as_deref().unwrap_or("public");
			self.alter(&current, format!("SET SCHEMA {}", ident(namespace)));
			current.namespace.clone_from(&to.namespace);
		}

		if from.name != to.name {
			self.alter(&current, format!("RENAME TO {}", ident(&to.name)));
		}

		// The primary key's name follows the table's, so that later steps
		// find it.
		let old_key = crate::constraint_name(from, &[], "pkey");
		let new_key = crate::constraint_name(to, &[], "pkey");

		if old_key != new_key {
			self.alter(
				to,
				format!(
					"RENAME CONSTRAINT {} TO {}",
					ident(&old_key),
					ident(&new_key)
				),
			);
		}
	}

	fn alter_column(&mut self, table: &TableName, from: &Column, to: &Column) {
		let column = ident(&to.name);

		if from.ty != to.ty {
			let ty = sql_type(&to.ty);
			self.alter(
				table,
				format!("ALTER COLUMN {column} TYPE {ty} USING {column}::{ty}"),
			);
		}

		if from.default != to.default {
			match &to.default {
				Some(default) => {
					self.alter(
						table,
						format!(
							"ALTER COLUMN {column} SET DEFAULT {}",
							default_expression(default)
						),
					);
				}
				None => self.alter(table, format!("ALTER COLUMN {column} DROP DEFAULT")),
			}
		}

		if from.nullable && !to.nullable {
			if let Some(default) = &to.default {
				self.statements.push(format!(
					"UPDATE {} SET {column} = {} WHERE {column} IS NULL",
					qualified(table),
					default_expression(default)
				));
			}

			self.alter(table, format!("ALTER COLUMN {column} SET NOT NULL"));
		} else if !from.nullable && to.nullable {
			self.alter(table, format!("ALTER COLUMN {column} DROP NOT NULL"));
		}
	}
}

fn view_keyword(view: &View) -> &'static str {
	if view.materialized {
		"MATERIALIZED VIEW"
	} else {
		"VIEW"
	}
}

/// `CREATE [MATERIALIZED] VIEW name (columns) AS query`, which names the
/// query's columns as declared, then its comment. A materialized view is
/// filled as it's created.
fn create_view(view: &View) -> Vec<String> {
	let names: Vec<String> = view
		.columns
		.iter()
		.map(|column| column.name.clone())
		.collect();
	let mut statements = vec![format!(
		"CREATE {} {} ({}) AS\n{}",
		view_keyword(view),
		qualified(&view.name),
		columns(&names),
		view.sql.postgres.trim()
	)];

	if let Some(comment) = &view.comment {
		statements.push(format!(
			"COMMENT ON {} {} IS {}",
			view_keyword(view),
			qualified(&view.name),
			literal(comment)
		));
	}

	statements
}

fn qualified(table: &TableName) -> String {
	match &table.namespace {
		Some(namespace) => format!("{}.{}", ident(namespace), ident(&table.name)),
		None => ident(&table.name),
	}
}

fn columns(columns: &[String]) -> String {
	columns
		.iter()
		.map(|column| ident(column))
		.collect::<Vec<_>>()
		.join(", ")
}

fn sql_type(ty: &ColumnType) -> String {
	match ty {
		ColumnType::Bool => "boolean".into(),
		ColumnType::Int16 => "smallint".into(),
		ColumnType::Int32 => "integer".into(),
		ColumnType::Int64 => "bigint".into(),
		ColumnType::Float32 => "real".into(),
		ColumnType::Float64 => "double precision".into(),
		ColumnType::Decimal { precision, scale } => {
			match (precision, scale) {
				(Some(precision), Some(scale)) => format!("numeric({precision}, {scale})"),
				(Some(precision), None) => format!("numeric({precision})"),
				_ => "numeric".into(),
			}
		}
		ColumnType::Text { max_len: Some(max) } => format!("varchar({max})"),
		ColumnType::Text { max_len: None } => "text".into(),
		ColumnType::Bytes => "bytea".into(),
		ColumnType::Uuid => "uuid".into(),
		ColumnType::Timestamp => "timestamptz".into(),
		ColumnType::Date => "date".into(),
		ColumnType::Json => "jsonb".into(),
		ColumnType::Array { element } => format!("{}[]", sql_type(element)),
	}
}

fn default_expression(default: &DefaultValue) -> String {
	match default {
		DefaultValue::Bool(value) => value.to_string(),
		DefaultValue::Integer(value) => value.to_string(),
		DefaultValue::Float(value) => float_literal(*value),
		DefaultValue::Text(value) => literal(value),
		DefaultValue::Now => "now()".into(),
		DefaultValue::Sql(sql) => format!("({sql})"),
	}
}

fn column_definition(column: &Column) -> String {
	let mut definition = format!("{} {}", ident(&column.name), sql_type(&column.ty));

	if !column.nullable {
		definition.push_str(" NOT NULL");
	}

	if let Some(default) = &column.default {
		definition.push_str(" DEFAULT ");
		definition.push_str(&default_expression(default));
	}

	if let Some(sql) = column.generated_sql(Dialect::Postgres) {
		definition.push_str(" GENERATED ALWAYS AS (");
		definition.push_str(sql);
		definition.push_str(") STORED");
	}

	definition
}

fn column_comment(table: &TableName, column: &str, comment: Option<&str>) -> String {
	let comment = comment.map_or_else(|| "NULL".to_string(), literal);
	format!(
		"COMMENT ON COLUMN {}.{} IS {comment}",
		qualified(table),
		ident(column)
	)
}

fn create_index(table: &TableName, index: &Index) -> String {
	let unique = if index.unique { "UNIQUE " } else { "" };
	let mut sql = format!(
		"CREATE {unique}INDEX {} ON {} ({})",
		ident(&index.name),
		qualified(table),
		columns(&index.columns)
	);

	if let Some(predicate) = &index.predicate {
		sql.push_str(" WHERE ");
		sql.push_str(&predicate_sql(predicate));
	}

	sql
}

pub(super) fn predicate_sql(predicate: &Predicate) -> String {
	match predicate {
		Predicate::IsNull(column) => format!("{} IS NULL", ident(column)),
		Predicate::IsNotNull(column) => format!("{} IS NOT NULL", ident(column)),
		Predicate::Sql(sql) => sql.clone(),
	}
}

fn add_foreign_key(table: &TableName, foreign_key: &ForeignKey) -> String {
	let mut sql = format!(
		"ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {}",
		qualified(table),
		ident(&foreign_key.name),
		columns(&foreign_key.columns),
		qualified(&foreign_key.references),
		columns(&foreign_key.referenced_columns),
		foreign_key.on_delete.sql()
	);

	if foreign_key.deferred {
		sql.push_str(" DEFERRABLE INITIALLY DEFERRED");
	}

	sql
}

fn check_constraint(check: &Check) -> String {
	format!(
		"CONSTRAINT {} CHECK ({})",
		ident(&check.name),
		check_expression(&check.rule)
	)
}

fn check_expression(rule: &CheckRule) -> String {
	match rule {
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
				"TRUE".into()
			} else {
				parts.join(" AND ")
			}
		}
		CheckRule::Length { column, min, max } => {
			let length = format!("char_length({})", ident(column));
			let mut parts = Vec::new();

			if let Some(min) = min {
				parts.push(format!("{length} >= {min}"));
			}

			if let Some(max) = max {
				parts.push(format!("{length} <= {max}"));
			}

			if parts.is_empty() {
				"TRUE".into()
			} else {
				parts.join(" AND ")
			}
		}
		CheckRule::OneOf { column, values } => {
			let values: Vec<String> = values.iter().map(|value| literal(value)).collect();
			format!("{} IN ({})", ident(column), values.join(", "))
		}
		CheckRule::Regex { column, pattern } => format!("{} ~ {}", ident(column), literal(pattern)),
		CheckRule::Sql { sql } => sql.clone(),
	}
}
