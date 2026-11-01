//! What the database changes on its own when rows are deleted for good:
//! `ON DELETE CASCADE` deletes the rows linking to them, and `SET NULL`
//! empties their links. The database layer never sees those rows, so
//! before such a delete it reads and locks them, through every cascade, and
//! once the delete has run it records them in the audit log, for the tables
//! that keep one. Live tables need none of this: the database fires their
//! capture triggers for the rows its cascades change.

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::OnceLock;

use leptoskit_db_schema::ReferentialAction;
use leptoskit_db_schema::TableName;

use crate::AuditOp;
use crate::Connection;
use crate::Result;
use crate::Table;
use crate::Value;
use crate::audit::Change;
use crate::audit::Subject;
use crate::sql::Sql;
use crate::table::TableEntry;

/// A foreign key that the database follows when the row it references is
/// deleted.
struct Edge {
	/// The table holding the key.
	source: &'static TableEntry,
	/// Its columns.
	columns: Vec<String>,
	/// The referenced table's columns they hold.
	referenced: Vec<String>,
	/// Whether the database empties the key rather than deleting its row.
	set_null: bool,
}

/// The cascading foreign keys into each table.
fn edges() -> &'static HashMap<TableName, Vec<Edge>> {
	static EDGES: OnceLock<HashMap<TableName, Vec<Edge>>> = OnceLock::new();
	EDGES.get_or_init(|| {
		let mut edges: HashMap<TableName, Vec<Edge>> = HashMap::new();

		for entry in crate::table::entries().values() {
			for foreign_key in (entry.definition)().foreign_keys {
				let set_null = match foreign_key.on_delete {
					ReferentialAction::Cascade => false,
					ReferentialAction::SetNull => true,
					ReferentialAction::NoAction | ReferentialAction::Restrict => continue,
				};
				edges.entry(foreign_key.references).or_default().push(Edge {
					source: entry,
					columns: foreign_key.columns,
					referenced: foreign_key.referenced_columns,
					set_null,
				});
			}
		}

		edges
	})
}

/// Whether deleting rows of `table` for good changes, through the
/// database's own cascades, rows of an audited table.
pub(crate) fn reaches<T: Table>() -> bool {
	static REACHES: OnceLock<HashMap<TableName, bool>> = OnceLock::new();
	let reaches = REACHES.get_or_init(|| {
		crate::table::entries()
			.keys()
			.map(|table| (table.clone(), reaches_recorded(table)))
			.collect()
	});
	reaches.get(&T::table_name()).copied().unwrap_or(false)
}

fn reaches_recorded(table: &TableName) -> bool {
	let mut seen = HashSet::new();
	let mut pending = vec![table.clone()];

	while let Some(table) = pending.pop() {
		for edge in edges().get(&table).into_iter().flatten() {
			let subject = &edge.source.subject;

			if subject.audit {
				return true;
			}

			let source = (edge.source.name)();

			if !edge.set_null && seen.insert(source.clone()) {
				pending.push(source);
			}
		}
	}

	false
}

/// The rows a delete will change on its own, read before it runs.
#[must_use = "the rows are only recorded by `Fallout::record`"]
pub(crate) struct Fallout {
	changes: Vec<(Subject, Vec<Change>)>,
}

/// The most values one statement binds while following cascades.
const BATCH: usize = 500;

/// Reads and locks every row the database will delete or change when the
/// rows of `table` given here, as values in `columns` order and already
/// locked, are deleted for good.
pub(crate) async fn before_delete(
	connection: &impl Connection,
	table: TableName,
	columns: &'static [&'static str],
	rows: Vec<Vec<Value>>,
) -> Result<Fallout> {
	let mut recorded: HashMap<TableName, (Subject, Vec<Change>)> = HashMap::new();
	// Keyed by their debug text: `Value` floats aren't `Hash`.
	let mut deleted: HashSet<(TableName, String)> = HashSet::new();

	if let Some(entry) = crate::table::entries().get(&table) {
		for row in &rows {
			deleted.insert((table.clone(), format!("{:?}", entry.subject.key_of(row))));
		}
	}

	let mut pending = vec![(table, columns, rows)];

	while let Some((table, columns, rows)) = pending.pop() {
		for edge in edges().get(&table).into_iter().flatten() {
			let subject = edge.source.subject;
			let source = (edge.source.name)();
			let records = subject.audit;

			if !records && (edge.set_null || !reaches_recorded(&source)) {
				continue;
			}

			let referenced: Vec<usize> = edge
				.referenced
				.iter()
				.filter_map(|name| columns.iter().position(|column| column == name))
				.collect();

			if referenced.len() != edge.referenced.len() {
				continue;
			}

			let targets: Vec<Vec<Value>> = rows
				.iter()
				.map(|row| referenced.iter().map(|&index| row[index].clone()).collect())
				.filter(|values: &Vec<Value>| !values.contains(&Value::Null))
				.collect();
			let mut reached = Vec::new();

			for batch in targets.chunks(BATCH) {
				for row in linked(connection, &source, subject.columns, edge, batch).await? {
					// Read back through the table's type, so keys compare
					// and record the same whichever statement read them.
					reached.push((subject.normalize)(row)?);
				}
			}

			let mut cascading = Vec::new();

			for row in reached {
				let key = format!("{:?}", subject.key_of(&row));

				if deleted.contains(&(source.clone(), key.clone())) {
					continue;
				}

				let change = if edge.set_null {
					emptied(edge, &subject, row)
				} else {
					deleted.insert((source.clone(), key));
					cascading.push(row.clone());
					Change {
						op: AuditOp::Delete,
						before: Some(row),
						after: None,
					}
				};

				if records {
					recorded
						.entry(source.clone())
						.or_insert_with(|| (subject, Vec::new()))
						.1
						.push(change);
				}
			}

			if !cascading.is_empty() {
				pending.push((source, subject.columns, cascading));
			}
		}
	}

	// A row both emptied and deleted is deleted.
	let changes = recorded
		.into_values()
		.map(|(subject, changes)| {
			let table = (subject.table)();
			let changes = changes
				.into_iter()
				.filter(|change| {
					change.op == AuditOp::Delete
						|| change.before.as_ref().is_none_or(|row| {
							!deleted
								.contains(&(table.clone(), format!("{:?}", subject.key_of(row))))
						})
				})
				.collect();
			(subject, changes)
		})
		.collect();

	Ok(Fallout { changes })
}

/// A row whose `edge` columns the database empties, before and after.
fn emptied(edge: &Edge, subject: &Subject, row: Vec<Value>) -> Change {
	let mut after = row.clone();

	for column in &edge.columns {
		if let Some(index) = subject.columns.iter().position(|name| name == column) {
			after[index] = Value::Null;
		}
	}

	Change {
		op: AuditOp::Update,
		before: Some(row),
		after: Some(after),
	}
}

/// The rows of `source` whose `edge` columns hold one of `targets`, locked
/// on Postgres until the transaction ends.
async fn linked(
	connection: &impl Connection,
	source: &TableName,
	columns: &[&str],
	edge: &Edge,
	targets: &[Vec<Value>],
) -> Result<Vec<Vec<Value>>> {
	let mut sql = Sql::new(connection.dialect());
	sql.push("SELECT ")
		.columns(columns)
		.push(" FROM ")
		.table_name(source)
		.push(" WHERE ");

	for (row, target) in targets.iter().enumerate() {
		sql.push(if row == 0 { "(" } else { " OR (" });

		for (index, (column, value)) in edge.columns.iter().zip(target).enumerate() {
			if index > 0 {
				sql.push(" AND ");
			}

			sql.identifier(column).push(" = ").value(value.clone());
		}

		sql.push(")");
	}

	if connection.dialect() == crate::Dialect::Postgres {
		sql.push(" FOR UPDATE");
	}

	connection.fetch(sql.finish()).await
}

impl Fallout {
	/// Records the rows, once the delete has changed them.
	pub(crate) async fn record(self, connection: &impl Connection) -> Result<()> {
		for (subject, changes) in self.changes {
			if !changes.is_empty() {
				crate::audit::record_changes(connection, subject, changes).await?;
			}
		}

		Ok(())
	}
}
