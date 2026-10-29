//! SQL text and its arguments, built for one dialect.

use std::fmt::Write;

use leptoskit_db_schema::Dialect;
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
}

impl Sql {
	pub(crate) fn new(dialect: Dialect) -> Self {
		Self {
			dialect,
			text: String::new(),
			args: Vec::new(),
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

	/// A table known only by name, such as one a cascade reaches.
	pub(crate) fn table_name(&mut self, table: &leptoskit_db_schema::TableName) -> &mut Self {
		match (self.dialect, &table.namespace) {
			(Dialect::Postgres, Some(namespace)) => {
				self.text.push_str(&quote_identifier(namespace));
				self.text.push('.');
				self.text.push_str(&quote_identifier(&table.name));
			}
			_ => self.text.push_str(&quote_identifier(&table.flat())),
		}

		self
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
