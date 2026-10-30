//! SQL text and its arguments, built for one dialect.

use std::fmt::Write;

use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::TableName;
use leptoskit_db_schema::quote_identifier;

use crate::Table;
use crate::Value;

/// One statement and the values bound to its placeholders.
#[derive(Clone, Debug, PartialEq)]
pub struct Statement {
	/// The SQL text, with dialect placeholders (`$1` or `?1`).
	pub sql: String,
	/// The values, in placeholder order.
	pub args: Vec<Value>,
}

/// Builds a statement while numbering placeholders for the dialect.
pub(crate) struct Sql {
	dialect: Dialect,
	text: String,
	args: Vec<Value>,
	/// The rows a condition can name, outermost first: the statement's
	/// table, then each count's, with the text that qualifies their columns.
	scopes: Vec<(TableName, String)>,
	/// The outermost scope the condition being written may name. A counted
	/// table's own policy reads its rows, never the rows around the count.
	origin: usize,
}

impl Sql {
	pub(crate) fn new(dialect: Dialect) -> Self {
		Self {
			dialect,
			text: String::new(),
			args: Vec::new(),
			scopes: Vec::new(),
			origin: 0,
		}
	}

	pub(crate) fn dialect(&self) -> Dialect {
		self.dialect
	}

	pub(crate) fn push(&mut self, text: &str) -> &mut Self {
		self.text.push_str(text);
		self
	}

	pub(crate) fn identifier(&mut self, name: &str) -> &mut Self {
		self.text.push_str(&quote_identifier(name));
		self
	}

	/// A placeholder bound to `value`.
	pub(crate) fn value(&mut self, value: Value) -> &mut Self {
		self.args.push(value);
		let number = self.args.len();

		let _ = match self.dialect {
			Dialect::Postgres => write!(self.text, "${number}"),
			Dialect::Sqlite => write!(self.text, "?{number}"),
		};

		self
	}

	/// The table's name: namespaced on Postgres, prefixed on SQLite.
	pub(crate) fn table<T: Table>(&mut self) -> &mut Self {
		self.table_name(&T::table_name())
	}

	/// A table known only by name, such as one a cascade reaches. The first
	/// table a statement names is the row its conditions are about.
	pub(crate) fn table_name(&mut self, table: &TableName) -> &mut Self {
		let name = self.render_table(table);
		self.text.push_str(&name);

		if self.scopes.is_empty() {
			self.scopes.push((table.clone(), name));
		}

		self
	}

	/// Makes `alias` the statement's outermost row, standing for a row of
	/// `table` that isn't stored in it, such as a change being checked
	/// against a policy: conditions name its columns through the alias.
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	pub(crate) fn root_alias(&mut self, table: &TableName, alias: &str) -> &mut Self {
		if self.scopes.is_empty() {
			self.scopes.push((table.clone(), quote_identifier(alias)));
		}

		self
	}

	fn render_table(&self, table: &TableName) -> String {
		match (self.dialect, &table.namespace) {
			(Dialect::Postgres, Some(namespace)) => {
				format!(
					"{}.{}",
					quote_identifier(namespace),
					quote_identifier(&table.name)
				)
			}
			_ => quote_identifier(&table.flat()),
		}
	}

	/// Opens a subquery's scope: writes `table AS lk_<kind>_<depth>`, so its
	/// columns can't be mistaken for those of the same table outside it.
	/// Returns the alias, quoted.
	pub(crate) fn enter(&mut self, table: &TableName, kind: &str) -> String {
		let (name, alias) = self.open(table, kind);
		let _ = write!(self.text, "{name} AS {alias}");
		alias
	}

	/// Opens a scope for `table` without writing anything, for a statement
	/// that names the scope's alias before its `FROM`. Returns the table's
	/// rendered name and the alias, both quoted.
	pub(crate) fn open(&mut self, table: &TableName, kind: &str) -> (String, String) {
		let alias = quote_identifier(&format!("lk_{kind}_{}", self.scopes.len()));
		let name = self.render_table(table);
		self.scopes.push((table.clone(), alias.clone()));
		(name, alias)
	}

	/// How many scopes are open: the depth the next one opens at.
	pub(crate) fn depth(&self) -> usize {
		self.scopes.len()
	}

	/// Closes the scope [`Sql::enter`] opened.
	pub(crate) fn leave(&mut self) {
		self.scopes.pop();
	}

	/// Writes a condition that may only name the innermost scope and the
	/// subqueries inside it, such as a counted table's own policy.
	pub(crate) fn isolated(
		&mut self,
		write: impl FnOnce(&mut Self) -> crate::Result<()>,
	) -> crate::Result<()> {
		let origin = std::mem::replace(&mut self.origin, self.scopes.len().saturating_sub(1));
		let written = write(self);
		self.origin = origin;
		written
	}

	/// A column of a row around the current one: the outermost row of
	/// `table` the condition being written may name. The innermost row's
	/// columns need no qualifier.
	pub(crate) fn outer_column(&mut self, table: &TableName, column: &str) -> crate::Result<()> {
		let Some(position) = self
			.scopes
			.iter()
			.skip(self.origin)
			.position(|(scope, _)| scope == table)
			.map(|position| position + self.origin)
		else {
			return Err(crate::expr::invalid(&format!(
				"`{}.{column}` belongs to neither the row being checked nor a table it counts",
				table.flat()
			)));
		};

		if position + 1 < self.scopes.len() {
			self.text.push_str(&self.scopes[position].1);
			self.text.push('.');
		}

		self.identifier(column);
		Ok(())
	}

	/// The columns, separated by commas.
	pub(crate) fn columns(&mut self, columns: &[&str]) -> &mut Self {
		for (index, column) in columns.iter().enumerate() {
			if index > 0 {
				self.text.push_str(", ");
			}

			self.identifier(column);
		}

		self
	}

	pub(crate) fn finish(self) -> Statement {
		Statement {
			sql: self.text,
			args: self.args,
		}
	}
}

/// The number a `SELECT count(*)` returned.
pub(crate) fn count_of(rows: &[Vec<Value>]) -> crate::Result<u64> {
	match rows.first().and_then(|row| row.first()) {
		Some(Value::Int(count)) => Ok(u64::try_from(*count).unwrap_or_default()),
		_ => {
			Err(crate::DbError::Decode(crate::DecodeError::custom(
				"count(*) returned no integer",
			)))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn counts_decode_from_the_first_column() {
		assert_eq!(count_of(&[vec![Value::Int(3)]]).unwrap(), 3);
		assert_eq!(count_of(&[vec![Value::Int(-1)]]).unwrap(), 0);
		assert!(count_of(&[]).is_err());
		assert!(count_of(&[vec![Value::Text("3".into())]]).is_err());
	}
}
