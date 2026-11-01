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
use crate::GeoIndex;
use crate::GeoPackageLayer;
use crate::Index;
use crate::Plan;
use crate::SEARCH_ROW;
use crate::Schema;
use crate::Search;
use crate::Step;
use crate::Table;
use crate::TableName;
use crate::Trigger;
use crate::TriggerAction;
use crate::VectorDistance;
use crate::VectorElement;
use crate::VectorOptions;
use crate::View;
use crate::quote_identifier as ident;
use crate::quote_literal as literal;
use crate::source_fingerprint_sql;

/// The format SQLite stores timestamps in: RFC 3339 in UTC with exactly three
/// fractional digits, so that text order is time order.
const NOW: &str = "(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))";

/// The prefix for the temporary table a rebuild copies rows into.
const REBUILD_PREFIX: &str = "_leptoskit_rebuild_";

#[expect(
	clippy::too_many_lines,
	reason = "one pass over the plan keeps the rebuild rules together"
)]
pub(super) fn render(plan: &Plan, previous: &Schema, target: &Schema) -> Migration {
	let before_renames = names_before_renames(plan);
	let rebuilt = tables_to_rebuild(plan, previous, &before_renames);
	let mut statements = Vec::new();
	let mut notes = Vec::new();
	let mut rebuilds_written = false;
	let mut created_views = Vec::new();
	let mut created_triggers = Vec::new();

	for step in &plan.steps {
		let table = step.table();

		// Triggers go around everything else: dropped first, so that no table
		// change trips over one, and created last, after any rebuild.
		match step {
			Step::DropTrigger(trigger) => {
				statements.push(format!("DROP TRIGGER {}", ident(&trigger.name)));
				continue;
			}
			Step::CreateTrigger(trigger) => {
				created_triggers.push(create_trigger(trigger));
				continue;
			}
			Step::CreateSequence(_)
			| Step::DropSequence(_)
			| Step::CreateFunction(_)
			| Step::ReplaceFunction { .. }
			| Step::DropFunction(_) => {
				unreachable!("validation keeps sequences and functions out of SQLite schemas")
			}
			_ => {}
		}

		// Rebuilds run after every rename, new table, and simple new column, so
		// their copies can read the new names.
		if !rebuilds_written && step_phase_reaches_rebuilds(step) {
			write_rebuilds(&rebuilt, plan, target, &mut statements, &mut notes);
			rebuilds_written = true;
		}

		if rebuilt.contains(table)
			&& !matches!(
				step,
				Step::RenameTable { .. }
					| Step::RenameColumn { .. }
					| Step::CreateSearch { .. }
					| Step::DropSearch { .. }
					| Step::CreateGeoIndex { .. }
					| Step::DropGeoIndex { .. }
					| Step::CreateVectorIndex { .. }
					| Step::DropVectorIndex { .. }
					| Step::CreateVectorSource { .. }
					| Step::DropVectorSource { .. }
					| Step::RegisterGeoPackage { .. }
					| Step::UnregisterGeoPackage { .. }
			) {
			continue;
		}

		match step {
			Step::DropSearch { .. } => {
				let table = before_renames.get(table).unwrap_or(table);
				statements.extend(drop_search(table));
			}
			Step::DropGeoIndex { index, .. } => {
				let table = before_renames.get(table).unwrap_or(table);
				statements.extend(drop_geo_index(table, index));
			}
			Step::CreateGeoIndex { index, .. } => {
				let table = target
					.table(table)
					.expect("a table gaining a spatial index is in the target schema");
				statements.extend(create_geo_index(table, index));
			}
			Step::DropVectorIndex { .. }
			| Step::DropVectorSource { .. }
			| Step::CreateVectorIndex { .. }
			| Step::CreateVectorSource { .. } => {
				statements.extend(vector_step(step, target, &before_renames));
			}
			Step::UnregisterGeoPackage { .. } => {
				let table = before_renames.get(table).unwrap_or(table);
				statements.extend(unregister_geopackage(table));
			}
			Step::RegisterGeoPackage { layer, .. } => {
				statements.extend(register_geopackage(table, layer));
			}
			Step::CreateSearch { search, .. } => {
				let table = target
					.table(table)
					.expect("a table gaining a search index is in the target schema");
				statements.extend(create_search(table, search));
			}
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
			| Step::RenameConstraint { .. }
			// Validation refuses row-level security on SQLite.
			| Step::CreateRowSecurity { .. }
			| Step::DropRowSecurity { .. } => {}
			Step::AlterColumn { .. }
			| Step::SetPrimaryKey { .. }
			| Step::AddForeignKey { .. }
			| Step::DropForeignKey { .. }
			| Step::AddCheck { .. }
			| Step::DropCheck { .. } => {
				unreachable!("`tables_to_rebuild` rebuilds every table with this step")
			}
			Step::DropTrigger(_)
			| Step::CreateTrigger(_)
			| Step::CreateSequence(_)
			| Step::DropSequence(_)
			| Step::CreateFunction(_)
			| Step::ReplaceFunction { .. }
			| Step::DropFunction(_) => unreachable!("handled before the table's steps"),
		}
	}

	if !rebuilds_written {
		write_rebuilds(&rebuilt, plan, target, &mut statements, &mut notes);
	}

	let (drops, drops_a_cycle) = drop_tables(plan);
	statements.extend(drops);
	statements.append(&mut created_views);
	statements.extend(drop_geopackage(previous, target));
	statements.append(&mut created_triggers);

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
		Step::DropTrigger(_)
			| Step::DropView(_)
			| Step::DropSearch { .. }
			| Step::DropGeoIndex { .. }
			| Step::DropVectorIndex { .. }
			| Step::DropVectorSource { .. }
			| Step::UnregisterGeoPackage { .. }
			| Step::DropRowSecurity { .. }
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
	let (added, altered) = rebuilt_columns(plan, table);
	let name = table.name.flat();
	let temporary = format!("{REBUILD_PREFIX}{name}");
	statements.push(create_table_named(&temporary, table, notes));

	let copied: Vec<&Column> = table
		.columns
		.iter()
		.filter(|column| column.generated.is_none())
		.collect();
	let columns: Vec<String> = copied.iter().map(|column| ident(&column.name)).collect();
	let values = copied_values(&copied, &added, &altered);

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

	recreate_triggers(table, plan, statements);
}

/// The columns `plan` adds and alters on `table`.
fn rebuilt_columns<'a>(
	plan: &'a Plan,
	table: &Table,
) -> (HashSet<&'a str>, HashMap<&'a str, (&'a Column, &'a Column)>) {
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

	(added, altered)
}

/// The value each copied column takes from the old table's row.
fn copied_values(
	copied: &[&Column],
	added: &HashSet<&str>,
	altered: &HashMap<&str, (&Column, &Column)>,
) -> Vec<String> {
	copied
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
		.collect()
}

/// Gives the rebuilt table back the triggers its old one had, except those
/// the plan creates again.
fn recreate_triggers(table: &Table, plan: &Plan, statements: &mut Vec<String>) {
	recreate_search(table, plan, statements);
	recreate_geo_indexes(table, plan, statements);
	recreate_vector_triggers(table, plan, statements);
}

/// Recreates a search index's triggers unless the plan makes them itself.
fn recreate_search(table: &Table, plan: &Plan, statements: &mut Vec<String>) {
	// Dropping the old table dropped its triggers; the search index's
	// companion tables, keyed by the table's key rather than its row ids,
	// still hold every row's words. A plan that recreates the index makes
	// them itself.
	let recreated = plan.steps.iter().any(
		|step| matches!(step, Step::CreateSearch { table: searched, .. } if searched == &table.name),
	);

	if let Some(search) = table.search.as_ref().filter(|_| !recreated) {
		statements.extend(search_triggers(table, search));
	}
}

/// Recreates each spatial index's triggers unless the plan makes them
/// itself.
fn recreate_geo_indexes(table: &Table, plan: &Plan, statements: &mut Vec<String>) {
	// Spatial indexes keep their boxes the same way.
	for index in &table.geo_indexes {
		let recreated = plan.steps.iter().any(|step| {
			matches!(
				step,
				Step::CreateGeoIndex { table: indexed, index: created }
					if indexed == &table.name && created == index
			)
		});

		if !recreated {
			statements.extend(geo_index_triggers(table, index));
		}
	}
}

/// Recreates vector indexes' and sources' triggers unless the plan makes
/// them itself.
fn recreate_vector_triggers(table: &Table, plan: &Plan, statements: &mut Vec<String>) {
	// So do vector indexes, and the fingerprints of vectors' sources live
	// in the table itself.
	for vector in &table.vectors {
		let recreated = |indexing: bool| {
			plan.steps.iter().any(|step| {
				match step {
					Step::CreateVectorIndex {
						table: indexed,
						vector: created,
					} => indexing && indexed == &table.name && created.column == vector.column,
					Step::CreateVectorSource {
						table: indexed,
						vector: created,
					} => !indexing && indexed == &table.name && created.column == vector.column,
					_ => false,
				}
			})
		};

		if vector.is_indexed(Dialect::Sqlite) && !recreated(true) {
			statements.extend(vector_index_triggers(table, vector));
		}

		if !vector.sources.is_empty() && !recreated(false) {
			statements.extend(vector_source_triggers(table, vector));
		}
	}
}

/// A spatial index over one column of `table`: an R*Tree of each row's
/// bounding box, a table numbering the rows by key (an R*Tree's ids are
/// integers, and row ids change when SQLite rebuilds a table), the triggers
/// that keep both current on every write, raw SQL's included, and the rows
/// already there. The boxes come from the GeoPackage header through
/// `ST_MinX` and its siblings, which the database layer registers on every
/// connection, as GeoPackage's own R*Tree extension expects.
fn create_geo_index(table: &Table, index: &GeoIndex) -> Vec<String> {
	let name = ident(&table.name.flat());
	let rtree = ident(&index.rtree_table(&table.name));
	let rows = ident(&index.rows_table(&table.name));
	let row = ident(SEARCH_ROW);
	let column = ident(&index.column);
	let mut definitions = vec![format!("{row} INTEGER PRIMARY KEY")];

	for key in &table.primary_key {
		let column = table.column(key).expect("validation checks the key");
		definitions.push(format!(
			"{} {} NOT NULL",
			ident(key),
			storage_type(&column.ty)
		));
	}

	definitions.push(format!(
		"CONSTRAINT {} UNIQUE ({})",
		ident(&crate::shorten_identifier(&format!(
			"{}_key",
			index.rows_table(&table.name)
		))),
		columns(&table.primary_key)
	));
	let keys = columns(&table.primary_key);
	let joined: Vec<String> = table
		.primary_key
		.iter()
		.map(|key| format!("\"lk_rows\".{} = {name}.{}", ident(key), ident(key)))
		.collect();
	let shape = format!("{name}.{column}");

	let mut statements = vec![
		format!(
			"CREATE TABLE {rows} (\n\t{}\n) STRICT",
			definitions.join(",\n\t")
		),
		format!("CREATE VIRTUAL TABLE {rtree} USING rtree(id, minx, maxx, miny, maxy)"),
		format!("INSERT INTO {rows} ({keys}) SELECT {keys} FROM {name}"),
		format!(
			"INSERT INTO {rtree} (id, minx, maxx, miny, maxy) SELECT \"lk_rows\".{row}, {} FROM \
			 {name} JOIN {rows} AS \"lk_rows\" ON {} WHERE {shape} IS NOT NULL AND NOT \
			 ST_IsEmpty({shape})",
			envelope(&shape),
			joined.join(" AND ")
		),
	];
	statements.extend(geo_index_triggers(table, index));
	statements
}

/// `ST_MinX(shape), ST_MaxX(shape), ST_MinY(shape), ST_MaxY(shape)`.
fn envelope(shape: &str) -> String {
	["ST_MinX", "ST_MaxX", "ST_MinY", "ST_MaxY"]
		.map(|function| format!("{function}({shape})"))
		.join(", ")
}

/// The triggers that keep a spatial index current when a row is inserted,
/// its shape or key changes, and it's deleted.
fn geo_index_triggers(table: &Table, index: &GeoIndex) -> Vec<String> {
	let name = ident(&table.name.flat());
	let rtree = ident(&index.rtree_table(&table.name));
	let rows = ident(&index.rows_table(&table.name));
	let row = ident(SEARCH_ROW);
	let keys = columns(&table.primary_key);
	let matching = |image: &str| {
		table
			.primary_key
			.iter()
			.map(|key| format!("{} = {image}.{}", ident(key), ident(key)))
			.collect::<Vec<_>>()
			.join(" AND ")
	};
	let insert_box = |image: &str| {
		let shape = format!("{image}.{}", ident(&index.column));
		format!(
			"INSERT INTO {rtree} (id, minx, maxx, miny, maxy) SELECT {row}, {} FROM {rows} WHERE \
			 {} AND {shape} IS NOT NULL AND NOT ST_IsEmpty({shape});",
			envelope(&shape),
			matching(image)
		)
	};
	let delete_box = |image: &str| {
		format!(
			"DELETE FROM {rtree} WHERE id = (SELECT {row} FROM {rows} WHERE {});",
			matching(image)
		)
	};

	let new_keys: Vec<String> = table
		.primary_key
		.iter()
		.map(|key| format!("new.{}", ident(key)))
		.collect();
	let set_keys: Vec<String> = table
		.primary_key
		.iter()
		.map(|key| format!("{} = new.{}", ident(key), ident(key)))
		.collect();
	let watched: Vec<String> = std::iter::once(index.column.clone())
		.chain(
			table
				.primary_key
				.iter()
				.filter(|key| **key != index.column)
				.cloned(),
		)
		.collect();

	vec![
		format!(
			"CREATE TRIGGER {} AFTER INSERT ON {name} BEGIN\n\tINSERT INTO {rows} ({keys}) VALUES \
			 ({});\n\t{}\nEND",
			ident(&index.trigger_name(&table.name, "insert")),
			new_keys.join(", "),
			insert_box("new")
		),
		format!(
			"CREATE TRIGGER {} AFTER UPDATE OF {} ON {name} BEGIN\n\t{}\n\tUPDATE {rows} SET {} \
			 WHERE {};\n\t{}\nEND",
			ident(&index.trigger_name(&table.name, "update")),
			columns(&watched),
			delete_box("old"),
			set_keys.join(", "),
			matching("old"),
			insert_box("new")
		),
		format!(
			"CREATE TRIGGER {} AFTER DELETE ON {name} BEGIN\n\t{}\n\tDELETE FROM {rows} WHERE \
			 {};\nEND",
			ident(&index.trigger_name(&table.name, "delete")),
			delete_box("old"),
			matching("old")
		),
	]
}

/// Drops a spatial index: its triggers, then its companion tables.
fn drop_geo_index(table: &TableName, index: &GeoIndex) -> Vec<String> {
	let mut statements: Vec<String> = ["insert", "update", "delete"]
		.into_iter()
		.map(|event| format!("DROP TRIGGER {}", ident(&index.trigger_name(table, event))))
		.collect();
	statements.push(format!("DROP TABLE {}", ident(&index.rtree_table(table))));
	statements.push(format!("DROP TABLE {}", ident(&index.rows_table(table))));
	statements
}

/// The column a vector column's `vec0` table keeps its vectors in.
const VEC_COLUMN: &str = "lk_vector";

/// A vector value as sqlite-vec reads it: floats as they're stored, bits
/// through `vec_bit`, which marks the bytes as bits.
fn vec_value(element: VectorElement, value: &str) -> String {
	match element {
		VectorElement::Bit => format!("vec_bit({value})"),
		_ => value.to_string(),
	}
}

/// The element and dimensions of a table's vector column.
fn vector_type(table: &Table, vector: &VectorOptions) -> (VectorElement, u32) {
	table
		.column(&vector.column)
		.and_then(|column| column.ty.vector())
		.expect("validation checks that vector options name a vector column")
}

/// A vector column's index on SQLite: a sqlite-vec `vec0` table holding
/// each row's vector, a table numbering the rows by key (a `vec0` table's
/// rows are numbered by integers, and row ids change when SQLite rebuilds a
/// table), the triggers that keep both current on every write, raw SQL's
/// included, and the rows already there. Searches scan every vector, which
/// `vec0` does quickly; it has no approximate index.
fn create_vector_index(table: &Table, vector: &VectorOptions) -> Vec<String> {
	let name = ident(&table.name.flat());
	let vectors = ident(&vector.vec_table(&table.name));
	let rows = ident(&vector.rows_table(&table.name));
	let row = ident(SEARCH_ROW);
	let column = format!("{name}.{}", ident(&vector.column));
	let (element, dimensions) = vector_type(table, vector);
	let definition = match (
		element,
		vector.distance.and_then(VectorDistance::sqlite_metric),
	) {
		(VectorElement::Bit, _) => format!("{VEC_COLUMN} bit[{dimensions}]"),

		(_, Some(metric)) => format!("{VEC_COLUMN} float[{dimensions}] distance_metric={metric}"),
		(_, None) => format!("{VEC_COLUMN} float[{dimensions}]"),
	};
	let keys = columns(&table.primary_key);
	let joined: Vec<String> = table
		.primary_key
		.iter()
		.map(|key| format!("\"lk_rows\".{} = {name}.{}", ident(key), ident(key)))
		.collect();

	let mut statements = vec![
		rows_table(table, &vector.rows_table(&table.name)),
		format!("CREATE VIRTUAL TABLE {vectors} USING vec0({definition})"),
		format!("INSERT INTO {rows} ({keys}) SELECT {keys} FROM {name}"),
		format!(
			"INSERT INTO {vectors} (rowid, {VEC_COLUMN}) SELECT \"lk_rows\".{row}, {} FROM {name} \
			 JOIN {rows} AS \"lk_rows\" ON {} WHERE {column} IS NOT NULL",
			vec_value(element, &column),
			joined.join(" AND ")
		),
	];
	statements.extend(vector_index_triggers(table, vector));
	statements
}

/// The table numbering a companion's rows by key: `lk_row`, then the key's
/// columns, unique together.
fn rows_table(table: &Table, rows: &str) -> String {
	let mut definitions = vec![format!("{} INTEGER PRIMARY KEY", ident(SEARCH_ROW))];

	for key in &table.primary_key {
		let column = table.column(key).expect("validation checks the key");
		definitions.push(format!(
			"{} {} NOT NULL",
			ident(key),
			storage_type(&column.ty)
		));
	}

	definitions.push(format!(
		"CONSTRAINT {} UNIQUE ({})",
		ident(&crate::shorten_identifier(&format!("{rows}_key"))),
		columns(&table.primary_key)
	));
	format!(
		"CREATE TABLE {} (\n\t{}\n) STRICT",
		ident(rows),
		definitions.join(",\n\t")
	)
}

/// The events a vector column's index has a trigger for: a row inserted or
/// updated without a vector, with one, and deleted.
const VECTOR_EVENTS: [&str; 5] = ["insert_null", "insert", "update_null", "update", "delete"];

/// The triggers that keep a vector column's index current when a row is
/// inserted, its vector or key changes, and it's deleted. A row with a
/// vector and one without take different triggers, whose conditions never
/// both hold, so their order doesn't matter: each inserts its vector with
/// `VALUES`, since sqlite-vec reads bits by the subtype `vec_bit` gives
/// them, which SQLite drops from a trigger's `INSERT … SELECT … WHERE`.
fn vector_index_triggers(table: &Table, vector: &VectorOptions) -> Vec<String> {
	let name = ident(&table.name.flat());
	let vectors = ident(&vector.vec_table(&table.name));
	let rows = ident(&vector.rows_table(&table.name));
	let row = ident(SEARCH_ROW);
	let column = ident(&vector.column);
	let keys = columns(&table.primary_key);
	let (element, _) = vector_type(table, vector);
	let insert_row = {
		let new_keys: Vec<String> = table
			.primary_key
			.iter()
			.map(|key| format!("new.{}", ident(key)))
			.collect();
		format!(
			"INSERT INTO {rows} ({keys}) VALUES ({});",
			new_keys.join(", ")
		)
	};

	let move_row = {
		let set_keys: Vec<String> = table
			.primary_key
			.iter()
			.map(|key| format!("{} = new.{}", ident(key), ident(key)))
			.collect();
		format!(
			"DELETE FROM {vectors} WHERE rowid = (SELECT {row} FROM {rows} WHERE {});\n\tUPDATE \
			 {rows} SET {} WHERE {};",
			key_matches(table, "old"),
			set_keys.join(", "),
			key_matches(table, "old")
		)
	};
	let insert_vector = format!(
		"INSERT INTO {vectors} (rowid, {VEC_COLUMN}) VALUES ((SELECT {row} FROM {rows} WHERE {}), \
		 {});",
		key_matches(table, "new"),
		vec_value(element, &format!("new.{column}"))
	);
	let watched: Vec<String> = std::iter::once(vector.column.clone())
		.chain(
			table
				.primary_key
				.iter()
				.filter(|key| **key != vector.column)
				.cloned(),
		)
		.collect();
	let watched = columns(&watched);
	let trigger = |event: &str, timing: &str, when: &str, body: &str| {
		format!(
			"CREATE TRIGGER {} AFTER {timing} ON {name} {when}BEGIN\n\t{body}\nEND",
			ident(&vector.trigger_name(&table.name, event))
		)
	};

	vec![
		trigger(
			"insert_null",
			"INSERT",
			&format!("WHEN new.{column} IS NULL "),
			&insert_row,
		),
		trigger(
			"insert",
			"INSERT",
			&format!("WHEN new.{column} IS NOT NULL "),
			&format!("{insert_row}\n\t{insert_vector}"),
		),
		trigger(
			"update_null",
			&format!("UPDATE OF {watched}"),
			&format!("WHEN new.{column} IS NULL "),
			&move_row,
		),
		trigger(
			"update",
			&format!("UPDATE OF {watched}"),
			&format!("WHEN new.{column} IS NOT NULL "),
			&format!("{move_row}\n\t{insert_vector}"),
		),
		trigger(
			"delete",
			"DELETE",
			"",
			&format!(
				"DELETE FROM {vectors} WHERE rowid = (SELECT {row} FROM {rows} WHERE \
				 {});\n\tDELETE FROM {rows} WHERE {};",
				key_matches(table, "old"),
				key_matches(table, "old")
			),
		),
	]
}

/// `key = image.key AND …` for each of the table's key columns.
fn key_matches(table: &Table, image: &str) -> String {
	table
		.primary_key
		.iter()
		.map(|key| format!("{} = {image}.{}", ident(key), ident(key)))
		.collect::<Vec<_>>()
		.join(" AND ")
}

/// The statements of a step that indexes a vector column or records its
/// sources, or stops: drops name the table as it was before renames.
fn vector_step(
	step: &Step,
	target: &Schema,
	before_renames: &HashMap<TableName, TableName>,
) -> Vec<String> {
	let table = step.table();
	let created = || {
		target
			.table(table)
			.expect("a table gaining a vector index is in the target schema")
	};
	let dropped = || before_renames.get(table).unwrap_or(table);

	match step {
		Step::CreateVectorIndex { vector, .. } => create_vector_index(created(), vector),
		Step::CreateVectorSource { vector, .. } => create_vector_source(created(), vector),
		Step::DropVectorIndex { vector, .. } => drop_vector_index(dropped(), vector),
		Step::DropVectorSource { vector, .. } => drop_vector_source(dropped(), vector),
		_ => Vec::new(),
	}
}

/// Drops a vector column's index: its triggers, then its companion tables.
fn drop_vector_index(table: &TableName, vector: &VectorOptions) -> Vec<String> {
	let mut statements: Vec<String> = VECTOR_EVENTS
		.into_iter()
		.map(|event| format!("DROP TRIGGER {}", ident(&vector.trigger_name(table, event))))
		.collect();
	statements.push(format!("DROP TABLE {}", ident(&vector.vec_table(table))));
	statements.push(format!("DROP TABLE {}", ident(&vector.rows_table(table))));
	statements
}

/// Starts recording which sources each row's vector was made from: the
/// vectors already there are taken as made from their rows' sources, and
/// then the triggers follow every write.
fn create_vector_source(table: &Table, vector: &VectorOptions) -> Vec<String> {
	let column = ident(&vector.column);
	let source = ident(&vector.source_column());
	let mut statements = vec![format!(
		"UPDATE {} SET {source} = {} WHERE {column} IS NOT NULL AND {source} IS NULL",
		ident(&table.name.flat()),
		source_fingerprint_sql(Dialect::Sqlite, None, &vector.sources)
	)];
	statements.extend(vector_source_triggers(table, vector));
	statements
}

/// The triggers that record a row's sources' fingerprint when its vector
/// is inserted, or changes without the write setting the fingerprint
/// itself.
fn vector_source_triggers(table: &Table, vector: &VectorOptions) -> Vec<String> {
	let name = ident(&table.name.flat());
	let column = ident(&vector.column);
	let source = ident(&vector.source_column());
	let fingerprint = source_fingerprint_sql(Dialect::Sqlite, Some("new"), &vector.sources);
	let matching = key_matches(table, "new");

	vec![
		format!(
			"CREATE TRIGGER {} AFTER INSERT ON {name} WHEN new.{source} IS NULL AND new.{column} \
			 IS NOT NULL BEGIN\n\tUPDATE {name} SET {source} = {fingerprint} WHERE \
			 {matching};\nEND",
			ident(&vector.source_trigger_name(&table.name, Some("insert")))
		),
		format!(
			"CREATE TRIGGER {} AFTER UPDATE OF {column} ON {name} WHEN new.{column} IS NOT \
			 old.{column} AND new.{source} IS old.{source} BEGIN\n\tUPDATE {name} SET {source} = \
			 CASE WHEN new.{column} IS NULL THEN NULL ELSE {fingerprint} END WHERE \
			 {matching};\nEND",
			ident(&vector.source_trigger_name(&table.name, Some("update")))
		),
	]
}

/// Stops recording a vector column's sources.
fn drop_vector_source(table: &TableName, vector: &VectorOptions) -> Vec<String> {
	["insert", "update"]
		.into_iter()
		.map(|event| {
			format!(
				"DROP TRIGGER {}",
				ident(&vector.source_trigger_name(table, Some(event)))
			)
		})
		.collect()
}

/// GeoPackage's definition of WGS 84, which its specification requires
/// every GeoPackage to hold.
const WGS84_WKT: &str = "GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS \
                         84\",6378137,298.257223563,AUTHORITY[\"EPSG\",\"7030\"]],AUTHORITY[\"\
                         EPSG\",\"6326\"]],PRIMEM[\"Greenwich\",0,AUTHORITY[\"EPSG\",\"8901\"]],\
                         UNIT[\"degree\",0.0174532925199433,AUTHORITY[\"EPSG\",\"9122\"]],\
                         AUTHORITY[\"EPSG\",\"4326\"]]";

/// GeoPackage's application id, `GPKG`, in the file's header.
const GEOPACKAGE_APPLICATION_ID: u32 = 0x4750_4B47;

/// The GeoPackage version the metadata follows, 1.4.0, in the file's
/// header.
const GEOPACKAGE_VERSION: u32 = 10400;

/// Registers `layer` as `table`'s GeoPackage layer, creating GeoPackage's
/// metadata tables first, as its specification defines them, when this is
/// the first. A coordinate system other than WGS 84 is recorded by its EPSG
/// code with an `undefined` definition; GIS tools look the code up.
fn register_geopackage(table: &TableName, layer: &GeoPackageLayer) -> Vec<String> {
	let name = literal(&table.flat());
	let srid = layer.srid;
	let mut statements = vec![
		"CREATE TABLE IF NOT EXISTS \"gpkg_spatial_ref_sys\" (\n\t\"srs_name\" TEXT NOT \
		 NULL,\n\t\"srs_id\" INTEGER NOT NULL PRIMARY KEY,\n\t\"organization\" TEXT NOT \
		 NULL,\n\t\"organization_coordsys_id\" INTEGER NOT NULL,\n\t\"definition\" TEXT NOT \
		 NULL,\n\t\"description\" TEXT\n)"
			.to_string(),
		"CREATE TABLE IF NOT EXISTS \"gpkg_contents\" (\n\t\"table_name\" TEXT NOT NULL PRIMARY \
		 KEY,\n\t\"data_type\" TEXT NOT NULL,\n\t\"identifier\" TEXT UNIQUE,\n\t\"description\" \
		 TEXT DEFAULT '',\n\t\"last_change\" DATETIME NOT NULL DEFAULT \
		 (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),\n\t\"min_x\" DOUBLE,\n\t\"min_y\" \
		 DOUBLE,\n\t\"max_x\" DOUBLE,\n\t\"max_y\" DOUBLE,\n\t\"srs_id\" INTEGER,\n\tCONSTRAINT \
		 \"fk_gc_r_srs_id\" FOREIGN KEY (\"srs_id\") REFERENCES \"gpkg_spatial_ref_sys\" \
		 (\"srs_id\")\n)"
			.to_string(),
		"CREATE TABLE IF NOT EXISTS \"gpkg_geometry_columns\" (\n\t\"table_name\" TEXT NOT \
		 NULL,\n\t\"column_name\" TEXT NOT NULL,\n\t\"geometry_type_name\" TEXT NOT \
		 NULL,\n\t\"srs_id\" INTEGER NOT NULL,\n\t\"z\" TINYINT NOT NULL,\n\t\"m\" TINYINT NOT \
		 NULL,\n\tCONSTRAINT \"pk_geom_cols\" PRIMARY KEY (\"table_name\", \
		 \"column_name\"),\n\tCONSTRAINT \"uk_gc_table_name\" UNIQUE \
		 (\"table_name\"),\n\tCONSTRAINT \"fk_gc_tn\" FOREIGN KEY (\"table_name\") REFERENCES \
		 \"gpkg_contents\" (\"table_name\"),\n\tCONSTRAINT \"fk_gc_srs\" FOREIGN KEY (\"srs_id\") \
		 REFERENCES \"gpkg_spatial_ref_sys\" (\"srs_id\")\n)"
			.to_string(),
		format!(
			"INSERT OR IGNORE INTO \"gpkg_spatial_ref_sys\" (\"srs_name\", \"srs_id\", \
			 \"organization\", \"organization_coordsys_id\", \"definition\", \"description\") \
			 VALUES ('Undefined cartesian SRS', -1, 'NONE', -1, 'undefined', 'undefined cartesian \
			 coordinate reference system'), ('Undefined geographic SRS', 0, 'NONE', 0, \
			 'undefined', 'undefined geographic coordinate reference system'), ('WGS 84 \
			 geodetic', {}, 'EPSG', {}, {}, 'longitude/latitude coordinates in decimal degrees on \
			 the WGS 84 spheroid')",
			crate::WGS84,
			crate::WGS84,
			literal(WGS84_WKT)
		),
	];

	if srid != crate::WGS84 {
		statements.push(format!(
			"INSERT OR IGNORE INTO \"gpkg_spatial_ref_sys\" (\"srs_name\", \"srs_id\", \
			 \"organization\", \"organization_coordsys_id\", \"definition\") VALUES \
			 ('EPSG:{srid}', {srid}, 'EPSG', {srid}, 'undefined')"
		));
	}

	statements.extend([
		format!(
			"INSERT INTO \"gpkg_contents\" (\"table_name\", \"data_type\", \"identifier\", \
			 \"srs_id\") VALUES ({name}, 'features', {name}, {srid})"
		),
		format!(
			"INSERT INTO \"gpkg_geometry_columns\" (\"table_name\", \"column_name\", \
			 \"geometry_type_name\", \"srs_id\", \"z\", \"m\") VALUES ({name}, {}, {}, {srid}, 0, \
			 0)",
			literal(&layer.column),
			literal(&layer.shape.name().to_uppercase())
		),
		format!("PRAGMA application_id = {GEOPACKAGE_APPLICATION_ID}"),
		format!("PRAGMA user_version = {GEOPACKAGE_VERSION}"),
	]);
	statements
}

/// Drops GeoPackage's metadata tables once no table is a layer any more,
/// as a fresh database without layers has none, and clears the file's
/// GeoPackage marks.
fn drop_geopackage(previous: &Schema, target: &Schema) -> Vec<String> {
	let layers = |schema: &Schema| {
		schema
			.tables
			.iter()
			.any(|table| schema.geopackage_layer(table).is_some())
	};

	if !layers(previous) || layers(target) {
		return Vec::new();
	}

	vec![
		"DROP TABLE \"gpkg_geometry_columns\"".to_string(),
		"DROP TABLE \"gpkg_contents\"".to_string(),
		"DROP TABLE \"gpkg_spatial_ref_sys\"".to_string(),
		"PRAGMA application_id = 0".to_string(),
		"PRAGMA user_version = 0".to_string(),
	]
}

/// Removes `table`'s GeoPackage registration.
fn unregister_geopackage(table: &TableName) -> Vec<String> {
	let name = literal(&table.flat());
	vec![
		format!("DELETE FROM \"gpkg_geometry_columns\" WHERE \"table_name\" = {name}"),
		format!("DELETE FROM \"gpkg_contents\" WHERE \"table_name\" = {name}"),
	]
}

/// The tokenizer that splits and folds words in `language`, which
/// validation limits to `simple` and `english`. Neither strips accents,
/// as Postgres's configurations of the same name don't.
fn tokenizer(language: &str) -> &'static str {
	if language == Search::ENGLISH {
		"porter unicode61 remove_diacritics 0"
	} else {
		"unicode61 remove_diacritics 0"
	}
}

/// The FTS5 column holding the `index`th searched column. FTS5 reserves
/// some names, such as `rank`, so the columns are numbered rather than
/// named after the table's.
pub(crate) fn search_text_column(index: usize) -> String {
	format!("c{index}")
}

/// The text a searched column is indexed as: never `NULL`, and without the
/// two private-use characters highlights mark matches with.
fn indexed_text(column: &str) -> String {
	format!("replace(replace(coalesce({column}, ''), char(57344), ''), char(57345), '')")
}

/// The search index of `table`: an FTS5 table holding a copy of the
/// searched text, a table numbering the searched rows by key, the triggers
/// that keep both in step with every write, raw SQL's included, and the
/// rows already there.
fn create_search(table: &Table, search: &Search) -> Vec<String> {
	let name = ident(&table.name.flat());
	let text = ident(&Search::text_table(&table.name));
	let rows = ident(&Search::rows_table(&table.name));
	let row = ident(SEARCH_ROW);
	let mut definitions = vec![format!("{row} INTEGER PRIMARY KEY")];

	for key in &table.primary_key {
		let column = table.column(key).expect("validation checks the key");
		definitions.push(format!(
			"{} {} NOT NULL",
			ident(key),
			storage_type(&column.ty)
		));
	}

	definitions.push(format!(
		"CONSTRAINT {} UNIQUE ({})",
		ident(&crate::shorten_identifier(&format!(
			"{}_key",
			Search::rows_table(&table.name)
		))),
		columns(&table.primary_key)
	));
	let text_columns: Vec<String> = (0..search.columns.len())
		.map(|index| ident(&search_text_column(index)))
		.collect();
	let keys = columns(&table.primary_key);
	let joined: Vec<String> = table
		.primary_key
		.iter()
		.map(|key| format!("\"lk_rows\".{} = {name}.{}", ident(key), ident(key)))
		.collect();
	let values: Vec<String> = search
		.columns
		.iter()
		.map(|column| indexed_text(&format!("{name}.{}", ident(&column.name))))
		.collect();

	let mut statements = vec![
		format!(
			"CREATE TABLE {rows} (\n\t{}\n) STRICT",
			definitions.join(",\n\t")
		),
		format!(
			"CREATE VIRTUAL TABLE {text} USING fts5({}, tokenize = {})",
			text_columns.join(", "),
			literal(tokenizer(&search.language))
		),
		format!("INSERT INTO {rows} ({keys}) SELECT {keys} FROM {name}"),
		format!(
			"INSERT INTO {text} (\"rowid\", {}) SELECT \"lk_rows\".{row}, {} FROM {name} JOIN \
			 {rows} AS \"lk_rows\" ON {}",
			text_columns.join(", "),
			values.join(", "),
			joined.join(" AND ")
		),
	];
	statements.extend(search_triggers(table, search));
	statements
}

/// The triggers that index a searched row when it's inserted, its searched
/// or key columns change, and it's deleted.
fn search_triggers(table: &Table, search: &Search) -> Vec<String> {
	let name = ident(&table.name.flat());
	let text = ident(&Search::text_table(&table.name));
	let rows = ident(&Search::rows_table(&table.name));
	let row = ident(SEARCH_ROW);
	let keys = columns(&table.primary_key);
	let text_columns: Vec<String> = (0..search.columns.len())
		.map(|index| ident(&search_text_column(index)))
		.collect();
	let text_columns = text_columns.join(", ");
	let matching = |image: &str| {
		table
			.primary_key
			.iter()
			.map(|key| format!("{} = {image}.{}", ident(key), ident(key)))
			.collect::<Vec<_>>()
			.join(" AND ")
	};

	let row_of = |image: &str| format!("(SELECT {row} FROM {rows} WHERE {})", matching(image));
	let values = |image: &str| {
		search
			.columns
			.iter()
			.map(|column| indexed_text(&format!("{image}.{}", ident(&column.name))))
			.collect::<Vec<_>>()
			.join(", ")
	};
	let insert_text = |image: &str| {
		format!(
			"INSERT INTO {text} (\"rowid\", {text_columns}) VALUES ({}, {});",
			row_of(image),
			values(image)
		)
	};
	let delete_text =
		|image: &str| format!("DELETE FROM {text} WHERE \"rowid\" = {};", row_of(image));
	let new_keys: Vec<String> = table
		.primary_key
		.iter()
		.map(|key| format!("new.{}", ident(key)))
		.collect();
	let set_keys: Vec<String> = table
		.primary_key
		.iter()
		.map(|key| format!("{} = new.{}", ident(key), ident(key)))
		.collect();
	let watched: Vec<String> = search
		.columns
		.iter()
		.map(|column| column.name.clone())
		.chain(
			table
				.primary_key
				.iter()
				.filter(|key| !search.reads(key))
				.cloned(),
		)
		.collect();

	vec![
		format!(
			"CREATE TRIGGER {} AFTER INSERT ON {name} BEGIN\n\tINSERT INTO {rows} ({keys}) VALUES \
			 ({});\n\t{}\nEND",
			ident(&Search::trigger_name(&table.name, "insert")),
			new_keys.join(", "),
			insert_text("new")
		),
		format!(
			"CREATE TRIGGER {} AFTER UPDATE OF {} ON {name} BEGIN\n\t{}\n\tUPDATE {rows} SET {} \
			 WHERE {};\n\t{}\nEND",
			ident(&Search::trigger_name(&table.name, "update")),
			columns(&watched),
			delete_text("old"),
			set_keys.join(", "),
			matching("old"),
			insert_text("new")
		),
		format!(
			"CREATE TRIGGER {} AFTER DELETE ON {name} BEGIN\n\t{}\n\tDELETE FROM {rows} WHERE \
			 {};\nEND",
			ident(&Search::trigger_name(&table.name, "delete")),
			delete_text("old"),
			matching("old")
		),
	]
}

/// Drops a search index: its triggers, then its companion tables.
fn drop_search(table: &TableName) -> Vec<String> {
	let mut statements: Vec<String> = ["insert", "update", "delete"]
		.into_iter()
		.map(|event| {
			format!(
				"DROP TRIGGER {}",
				ident(&Search::trigger_name(table, event))
			)
		})
		.collect();
	statements.push(format!("DROP TABLE {}", ident(&Search::text_table(table))));
	statements.push(format!("DROP TABLE {}", ident(&Search::rows_table(table))));
	statements
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

/// `CREATE TRIGGER … FOR EACH ROW [WHEN …] BEGIN … END`.
fn create_trigger(trigger: &Trigger) -> String {
	let TriggerAction::Statements { when, sql } = &trigger.action else {
		unreachable!("validation keeps functions out of SQLite triggers")
	};
	let events: Vec<&str> = trigger.events.iter().map(|event| event.sql()).collect();
	let when = when
		.as_ref()
		.map(|condition| format!(" WHEN {condition}"))
		.unwrap_or_default();

	format!(
		"CREATE TRIGGER {} {} {} ON {} FOR EACH ROW{when} BEGIN\n\t{}\nEND",
		ident(&trigger.name),
		trigger.timing.sql(),
		events.join(" OR "),
		ident(&trigger.table.flat()),
		sql.trim()
	)
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
		ColumnType::Bytes
		| ColumnType::Uuid
		| ColumnType::Geometry { .. }
		| ColumnType::Geography { .. }
		| ColumnType::Vector { .. } => "BLOB",
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
		// A GeoPackage geometry of the declared shape and coordinate system,
		// with an accurate bounding box (which spatial indexes read), and
		// coordinates in range for geography.
		ColumnType::Geometry { shape, srid } => {
			Some(format!(
				"lk_geo_check({name}, {}, {srid}, 0)",
				literal(shape.name())
			))
		}
		ColumnType::Geography { shape } => {
			Some(format!(
				"lk_geo_check({name}, {}, {}, 1)",
				literal(shape.name()),
				crate::WGS84
			))
		}
		// sqlite-vec's layout: four bytes a float, one byte eight bits.
		ColumnType::Vector {
			element,
			dimensions,
		} => {
			element
				.sqlite_bytes(*dimensions)
				.map(|bytes| format!("length({name}) = {bytes}"))
		}
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
		CheckRule::ValidGeometry { column } => format!("lk_st_isvalid({})", ident(column)),
		CheckRule::Sql { sql } => sql.clone(),
	};

	Some(format!(
		"CONSTRAINT {} CHECK ({expression})",
		ident(&check.name)
	))
}
