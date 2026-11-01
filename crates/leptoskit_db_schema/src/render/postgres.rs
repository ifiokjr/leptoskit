//! PostgreSQL: every step is an `ALTER`, in plan order, except that foreign
//! keys on new tables are added once every table exists.

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fmt::Write;

use super::Migration;
use super::float_literal;
use super::names_before_renames;
use super::number_literal;
use crate::BODY_QUOTE;
use crate::BYPASS_SETTING;
use crate::Check;
use crate::CheckRule;
use crate::Column;
use crate::ColumnType;
use crate::DefaultValue;
use crate::Dialect;
use crate::ForeignKey;
use crate::Function;
use crate::Index;
use crate::Plan;
use crate::Predicate;
use crate::RowSecurity;
use crate::SEARCH_COLUMN;
use crate::Schema;
use crate::Search;
use crate::Step;
use crate::Table;
use crate::TableName;
use crate::Trigger;
use crate::TriggerAction;
use crate::VectorDistance;
use crate::VectorElement;
use crate::VectorIndexKind;
use crate::VectorOptions;
use crate::View;
use crate::quote_identifier as ident;
use crate::quote_literal as literal;
use crate::source_fingerprint_sql;

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
			.chain(schema.sequences.iter().map(|sequence| &sequence.name))
			.chain(schema.functions.iter().map(|function| &function.name))
			.filter_map(|name| name.namespace.clone())
			.collect()
	};
	let existing = namespaces(previous);
	let wanted = namespaces(target);
	let installed = previous.extensions();

	for extension in target.extensions() {
		if !installed.contains(&extension) {
			writer.statements.push(format!(
				"CREATE EXTENSION IF NOT EXISTS {}",
				ident(extension)
			));
		}
	}

	for namespace in wanted.difference(&existing) {
		writer
			.statements
			.push(format!("CREATE SCHEMA IF NOT EXISTS {}", ident(namespace)));
	}

	let mut pending_foreign_keys = Vec::new();
	let mut dropped_tables = Vec::new();
	let mut dropped_routines = Vec::new();
	let mut created_views = Vec::new();
	// A dropped table's search column goes with it.
	let dropping: BTreeSet<&TableName> = plan
		.steps
		.iter()
		.filter_map(|step| {
			match step {
				Step::DropTable(table) => Some(&table.name),
				_ => None,
			}
		})
		.collect();
	let mut created_triggers = Vec::new();

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
			Step::DropSearch { table, .. }
			| Step::DropGeoIndex { table, .. }
			| Step::DropVectorIndex { table, .. }
				if dropping.contains(table) => {}
			Step::CreateVectorIndex { table, vector } => {
				let element = target
					.table(table)
					.and_then(|table| table.column(&vector.column))
					.and_then(|column| column.ty.vector())
					.map(|(element, _)| element);
				writer.create_vector_index(table, vector, element);
			}
			Step::DropRowSecurity { table, security } => {
				let table = writer.name_for(step, table).clone();
				let reenabled = target
					.table(step.table())
					.is_some_and(|table| table.row_security.is_some());
				writer.drop_row_security(&table, security, !reenabled);
			}
			Step::DropFunction(function) => {
				dropped_routines.push(format!("DROP FUNCTION {}()", qualified(&function.name)));
			}
			Step::DropSequence(sequence) => {
				dropped_routines.push(format!("DROP SEQUENCE {}", qualified(&sequence.name)));
			}
			Step::CreateTrigger(trigger) => created_triggers.push(create_trigger(trigger)),
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

	writer.statements.append(&mut dropped_routines);
	writer.statements.append(&mut created_views);
	writer.statements.append(&mut created_triggers);

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
			Step::DropForeignKey { .. }
			| Step::DropIndex { .. }
			| Step::DropCheck { .. }
			| Step::DropSearch { .. }
			| Step::DropGeoIndex { .. }
			| Step::DropVectorIndex { .. }
			| Step::DropVectorSource { .. }
			| Step::DropRowSecurity { .. } => self.before_renames.get(table).unwrap_or(table),
			_ => table,
		}
	}

	/// Drops each policy, then turns row-level security off when nothing
	/// replaces them.
	fn drop_row_security(&mut self, table: &TableName, security: &RowSecurity, disable: bool) {
		for policy in &security.policies {
			self.statements.push(format!(
				"DROP POLICY {} ON {}",
				ident(&policy.name(table)),
				qualified(table)
			));
		}

		if disable {
			self.alter(table, "NO FORCE ROW LEVEL SECURITY");
			self.alter(table, "DISABLE ROW LEVEL SECURITY");
		}
	}

	/// Turns row-level security on, forced so the table's owner, which apps
	/// usually connect as, obeys it too, and creates each policy. Each
	/// admits a connection that bypasses policies.
	fn create_row_security(&mut self, table: &TableName, security: &RowSecurity) {
		self.alter(table, "ENABLE ROW LEVEL SECURITY");
		self.alter(table, "FORCE ROW LEVEL SECURITY");
		let bypass = format!("current_setting({}, true) = 'on'", literal(BYPASS_SETTING));

		for policy in &security.policies {
			let mut sql = format!(
				"CREATE POLICY {} ON {} FOR {}",
				ident(&policy.name(table)),
				qualified(table),
				policy.command.keyword()
			);

			if let Some(using) = &policy.using {
				let _ = write!(sql, " USING ({bypass} OR ({using}))");
			}

			if let Some(check) = &policy.check {
				let _ = write!(sql, " WITH CHECK ({bypass} OR ({check}))");
			}

			self.statements.push(sql);
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
			Step::CreateTable(_)
			| Step::DropTable(_)
			| Step::CreateView(_)
			| Step::DropView(_)
			| Step::DropFunction(_)
			| Step::DropSequence(_)
			| Step::CreateTrigger(_)
			| Step::DropRowSecurity { .. }
			| Step::CreateVectorIndex { .. } => {
				unreachable!("handled by `render`")
			}
			Step::CreateSequence(sequence) => {
				self.statements
					.push(format!("CREATE SEQUENCE {}", qualified(&sequence.name)));
			}
			Step::CreateFunction(function) | Step::ReplaceFunction { to: function, .. } => {
				self.statements.push(create_function(function));
			}
			// The trigger as it was, on its table's old name: drops run
			// before renames.
			Step::DropTrigger(trigger) => {
				self.statements.push(format!(
					"DROP TRIGGER {} ON {}",
					ident(&trigger.name),
					qualified(&trigger.table)
				));
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
			Step::CreateRowSecurity { security, .. } => self.create_row_security(&table, security),
			// The index goes with its column.
			Step::DropSearch { .. } => {
				self.alter(&table, format!("DROP COLUMN {}", ident(SEARCH_COLUMN)));
			}
			Step::DropGeoIndex { index, .. } => {
				let name = ident(&index.index_name(&table));
				let name = match &table.namespace {
					Some(namespace) => format!("{}.{name}", ident(namespace)),
					None => name,
				};
				self.statements.push(format!("DROP INDEX {name}"));
			}
			Step::CreateGeoIndex { index, .. } => {
				self.statements.push(format!(
					"CREATE INDEX {} ON {} USING gist ({})",
					ident(&index.index_name(&table)),
					qualified(&table),
					ident(&index.column)
				));
			}
			Step::DropVectorIndex { vector, .. } => {
				let name = ident(&vector.index_name(&table));
				let name = match &table.namespace {
					Some(namespace) => format!("{}.{name}", ident(namespace)),
					None => name,
				};
				self.statements.push(format!("DROP INDEX {name}"));
			}
			Step::CreateVectorSource { vector, .. } => self.create_vector_source(&table, vector),
			Step::DropVectorSource { vector, .. } => self.drop_vector_source(&table, vector),
			// Validation keeps GeoPackage metadata to SQLite.
			Step::RegisterGeoPackage { .. } | Step::UnregisterGeoPackage { .. } => {}
			Step::CreateSearch { search, .. } => self.create_search(&table, search),
		}
	}

	/// Drops the trigger and the function that kept a column's vector fresh.
	fn drop_vector_source(&mut self, table: &TableName, vector: &VectorOptions) {
		let name = vector.source_trigger_name(table, None);
		self.statements.push(format!(
			"DROP TRIGGER {} ON {}",
			ident(&name),
			qualified(table)
		));
		self.statements.push(format!(
			"DROP FUNCTION {}()",
			qualified(&TableName {
				namespace: table.namespace.clone(),
				name,
			})
		));
	}

	/// The generated `tsvector` column and the index over it.
	fn create_search(&mut self, table: &TableName, search: &Search) {
		self.alter(
			table,
			format!(
				"ADD COLUMN {} tsvector GENERATED ALWAYS AS ({}) STORED",
				ident(SEARCH_COLUMN),
				search_vector(search)
			),
		);
		self.statements.push(format!(
			"CREATE INDEX {} ON {} USING gin ({})",
			ident(&Search::index_name(table)),
			qualified(table),
			ident(SEARCH_COLUMN)
		));
	}

	/// pgvector's index over a vector column, with the operator class for
	/// its distance. IVFFlat takes pgvector's suggested 100 lists to start.
	fn create_vector_index(
		&mut self,
		table: &TableName,
		vector: &VectorOptions,
		element: Option<VectorElement>,
	) {
		let (Some(index), Some(distance), Some(element)) = (vector.index, vector.distance, element)
		else {
			return;
		};
		let operators = match distance {
			VectorDistance::L2 => "l2",
			VectorDistance::Cosine => "cosine",
			VectorDistance::InnerProduct => "ip",
			VectorDistance::L1 => "l1",
			VectorDistance::Hamming => "hamming",
			VectorDistance::Jaccard => "jaccard",
		};
		let options = match index {
			VectorIndexKind::Hnsw => "",
			VectorIndexKind::IvfFlat => " WITH (lists = 100)",
		};
		self.statements.push(format!(
			"CREATE INDEX {} ON {} USING {} ({} {}_{operators}_ops){options}",
			ident(&vector.index_name(table)),
			qualified(table),
			index.name(),
			ident(&vector.column),
			element.postgres_name()
		));
	}

	/// The trigger that records which sources each row's vector was made
	/// from: before a row is written, when its vector changes and the write
	/// doesn't set the fingerprint itself, the fingerprint of the row's
	/// sources as written. The vectors already there are taken as made from
	/// their rows' sources.
	fn create_vector_source(&mut self, table: &TableName, vector: &VectorOptions) {
		let name = vector.source_trigger_name(table, None);
		let function = qualified(&TableName {
			namespace: table.namespace.clone(),
			name: name.clone(),
		});
		let column = ident(&vector.column);
		let source = ident(&vector.source_column());
		let fingerprint = |row| source_fingerprint_sql(Dialect::Postgres, row, &vector.sources);

		self.statements.push(format!(
			"UPDATE {} SET {source} = {} WHERE {column} IS NOT NULL AND {source} IS NULL",
			qualified(table),
			fingerprint(None)
		));
		self.statements.push(format!(
			"CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $lk$\nBEGIN\n\tIF \
			 TG_OP = 'INSERT' THEN\n\t\tIF NEW.{source} IS NULL AND NEW.{column} IS NOT NULL \
			 THEN\n\t\t\tNEW.{source} := {new};\n\t\tEND IF;\n\tELSIF NEW.{column} IS DISTINCT \
			 FROM OLD.{column} AND NEW.{source} IS NOT DISTINCT FROM OLD.{source} \
			 THEN\n\t\tNEW.{source} := CASE WHEN NEW.{column} IS NULL THEN NULL ELSE {new} \
			 END;\n\tEND IF;\n\tRETURN NEW;\nEND\n$lk$",
			new = fingerprint(Some("NEW"))
		));
		self.statements.push(format!(
			"CREATE TRIGGER {} BEFORE INSERT OR UPDATE OF {column} ON {} FOR EACH ROW EXECUTE \
			 FUNCTION {function}()",
			ident(&name),
			qualified(table)
		));
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

/// The words of a searched row, each column's weighted by its letter:
/// `setweight(to_tsvector('simple'::regconfig, coalesce("title", '')), 'A')
/// || …`. A configuration given as a constant makes `to_tsvector`
/// immutable, as a generated column needs.
fn search_vector(search: &Search) -> String {
	search
		.columns
		.iter()
		.map(|column| {
			format!(
				"setweight(to_tsvector({}::regconfig, coalesce({}, '')), '{}')",
				literal(&search.language),
				ident(&column.name),
				column.weight.letter()
			)
		})
		.collect::<Vec<_>>()
		.join(" || ")
}

/// `CREATE OR REPLACE FUNCTION`. A function that runs with its owner's
/// privileges fixes its search path, so that nobody can slip their own
/// objects in front of the ones it names.
fn create_function(function: &Function) -> String {
	let security = if function.security_definer {
		" SECURITY DEFINER SET search_path = pg_catalog, pg_temp"
	} else {
		""
	};

	format!(
		"CREATE OR REPLACE FUNCTION {}() RETURNS {} LANGUAGE {}{security} AS \
		 {BODY_QUOTE}\n{}\n{BODY_QUOTE}",
		qualified(&function.name),
		function.returns,
		function.language,
		function.body.trim()
	)
}

/// `CREATE TRIGGER`, or `CREATE CONSTRAINT TRIGGER … DEFERRABLE INITIALLY
/// DEFERRED`, calling its function for each row.
fn create_trigger(trigger: &Trigger) -> String {
	let TriggerAction::Function {
		function,
		arguments,
	} = &trigger.action
	else {
		unreachable!("validation keeps statements out of Postgres triggers")
	};
	let constraint = if trigger.deferred { "CONSTRAINT " } else { "" };
	let deferred = if trigger.deferred {
		" DEFERRABLE INITIALLY DEFERRED"
	} else {
		""
	};
	let events: Vec<&str> = trigger.events.iter().map(|event| event.sql()).collect();
	let arguments: Vec<String> = arguments.iter().map(|argument| literal(argument)).collect();

	format!(
		"CREATE {constraint}TRIGGER {} {} {} ON {}{deferred} FOR EACH ROW EXECUTE FUNCTION {}({})",
		ident(&trigger.name),
		trigger.timing.sql(),
		events.join(" OR "),
		qualified(&trigger.table),
		qualified(function),
		arguments.join(", ")
	)
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
	let options = if view.security_invoker {
		" WITH (security_invoker = true)"
	} else {
		""
	};
	let mut statements = vec![format!(
		"CREATE {} {} ({}){options} AS\n{}",
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

	statements.extend(
		view.indexes
			.iter()
			.map(|index| create_index(&view.name, index)),
	);
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

/// The Postgres type a column of `ty` is stored as, such as `uuid` or
/// `varchar(80)`.
pub fn sql_type(ty: &ColumnType) -> String {
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
		ColumnType::Geometry { shape, srid } => format!("geometry({shape}, {srid})"),
		ColumnType::Geography { shape } => format!("geography({shape}, {})", crate::WGS84),
		ColumnType::Vector {
			element,
			dimensions,
		} => format!("{}({dimensions})", element.postgres_name()),
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
		CheckRule::ValidGeometry { column } => format!("ST_IsValid({}::geometry)", ident(column)),
		CheckRule::Sql { sql } => sql.clone(),
	}
}
