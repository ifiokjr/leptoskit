//! Typed columns, and the filters and orderings built from them.

use std::marker::PhantomData;
use std::ops::BitAnd;
use std::ops::BitOr;
use std::ops::Not;

use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::TableName;
use leptoskit_db_schema::quote_identifier;

use crate::Column;
use crate::DbError;
use crate::EncodeError;
use crate::Table;
use crate::Value;
use crate::sql::Sql;

/// One column of table `T`, holding values of type `V`. `#[db::table]`
/// generates one per field, so filters are checked against the column's type:
///
/// ```ignore
/// Post::query().filter(|p| p.author_id.eq(user.id) & p.published_at.is_not_null())
/// ```
pub struct Col<T, V> {
	name: &'static str,
	types: PhantomData<fn() -> (T, V)>,
}

impl<T, V> Col<T, V> {
	#[doc(hidden)]
	pub const fn new(name: &'static str) -> Self {
		Self {
			name,
			types: PhantomData,
		}
	}

	/// The column's name in the database.
	pub const fn name(&self) -> &'static str {
		self.name
	}
}

impl<T, V> Clone for Col<T, V> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<T, V> Copy for Col<T, V> {}

impl<T: Table, V: Column> Col<T, V> {
	/// The column equals `value`; comparing with `None` matches `NULL`.
	pub fn eq(&self, value: impl Operand<V>) -> Expr {
		self.compare(Comparison::Equal, value)
	}

	/// The column differs from `value`; comparing with `None` matches
	/// non-`NULL`.
	pub fn ne(&self, value: impl Operand<V>) -> Expr {
		self.compare(Comparison::NotEqual, value)
	}

	/// The column is greater than `value`.
	pub fn gt(&self, value: impl Operand<V>) -> Expr {
		self.compare(Comparison::Greater, value)
	}

	/// The column is at least `value`.
	pub fn gte(&self, value: impl Operand<V>) -> Expr {
		self.compare(Comparison::GreaterOrEqual, value)
	}

	/// The column is less than `value`.
	pub fn lt(&self, value: impl Operand<V>) -> Expr {
		self.compare(Comparison::Less, value)
	}

	/// The column is at most `value`.
	pub fn lte(&self, value: impl Operand<V>) -> Expr {
		self.compare(Comparison::LessOrEqual, value)
	}

	/// The column is one of `values`. An empty list matches nothing.
	pub fn in_list<O: Operand<V>>(&self, values: impl IntoIterator<Item = O>) -> Expr {
		match values
			.into_iter()
			.map(Operand::into_value)
			.collect::<Result<Vec<_>, _>>()
		{
			Ok(values) => {
				Expr::In {
					column: self.name,
					values,
				}
			}
			Err(error) => Expr::Invalid(error.to_string()),
		}
	}

	/// The column is `NULL`.
	pub fn is_null(&self) -> Expr {
		Expr::Null {
			column: self.name,
			negated: false,
		}
	}

	/// The column isn't `NULL`.
	pub fn is_not_null(&self) -> Expr {
		Expr::Null {
			column: self.name,
			negated: true,
		}
	}

	/// Ascending order.
	pub fn asc(&self) -> Order {
		Order {
			column: self.name,
			descending: false,
		}
	}

	/// Descending order.
	pub fn desc(&self) -> Order {
		Order {
			column: self.name,
			descending: true,
		}
	}

	fn compare(&self, comparison: Comparison, value: impl Operand<V>) -> Expr {
		match value.setting() {
			Some(Ok(setting)) => {
				return Expr::CompareSetting {
					column: self.name,
					comparison,
					setting,
				};
			}
			Some(Err(error)) => return Expr::Invalid(error),
			None => {}
		}

		if let Some((table, other)) = value.column() {
			return Expr::CompareColumn {
				column: self.name,
				comparison,
				table,
				other,
			};
		}

		match value.into_value() {
			Ok(value) => {
				Expr::Compare {
					column: self.name,
					comparison,
					value,
				}
			}
			Err(error) => Expr::Invalid(error.to_string()),
		}
	}
}

/// Text matching, for text columns.
impl<T: Table, V: Column + TextColumn> Col<T, V> {
	/// The text contains `needle`, case-sensitively.
	pub fn contains(&self, needle: &str) -> Expr {
		Expr::Contains {
			column: self.name,
			needle: needle.to_string(),
		}
	}

	/// The text matches a SQL `LIKE` pattern (`%` for any run, `_` for one
	/// character), ignoring case. SQLite folds ASCII letters only.
	pub fn ilike(&self, pattern: &str) -> Expr {
		Expr::Like {
			column: self.name,
			pattern: pattern.to_string(),
		}
	}
}

/// A column type that holds text.
#[diagnostic::on_unimplemented(
	message = "`{Self}` isn't text",
	label = "text matching and `#[db(search)]` need a text column, such as `String`"
)]
pub trait TextColumn {}

impl TextColumn for String {}
impl<T: TextColumn> TextColumn for Option<T> {}

/// A value a column of type `V` can be compared with: `V` itself, a
/// reference to it, the inside of an optional column, or `&str` for text.
///
/// Inside a policy's `cx.count(…)`, a column of the row the policy checks
/// works too, which is how a count follows a link back to that row:
///
/// ```ignore
/// cx.count(Member::query().filter(|member| member.team_id.eq(project.team_id)))
/// ```
pub trait Operand<V> {
	#[doc(hidden)]
	fn into_value(self) -> Result<Value, EncodeError>;

	/// The table and name of the column this operand is, when it's one.
	#[doc(hidden)]
	fn column(&self) -> Option<(TableName, &'static str)> {
		None
	}

	/// The request setting this operand is, when it's one.
	#[doc(hidden)]
	fn setting(&self) -> Option<Result<crate::settings::SettingRef, String>> {
		None
	}
}

/// A column compared with another column, which only a filter can do: a
/// column has no value of its own to write or list.
fn column_operand<U: Table>(name: &'static str) -> (TableName, &'static str) {
	(U::table_name(), name)
}

fn column_as_value(name: &str) -> EncodeError {
	EncodeError::new(format!(
		"column `{name}` has no value of its own; compare with it in a filter instead"
	))
}

impl<U: Table, V: Column> Operand<V> for Col<U, V> {
	fn into_value(self) -> Result<Value, EncodeError> {
		Err(column_as_value(self.name))
	}

	fn column(&self) -> Option<(TableName, &'static str)> {
		Some(column_operand::<U>(self.name))
	}
}

impl<U: Table, V: Column> Operand<Option<V>> for Col<U, V> {
	fn into_value(self) -> Result<Value, EncodeError> {
		Err(column_as_value(self.name))
	}

	fn column(&self) -> Option<(TableName, &'static str)> {
		Some(column_operand::<U>(self.name))
	}
}

impl<U: Table, V: Column> Operand<V> for Col<U, Option<V>> {
	fn into_value(self) -> Result<Value, EncodeError> {
		Err(column_as_value(self.name))
	}

	fn column(&self) -> Option<(TableName, &'static str)> {
		Some(column_operand::<U>(self.name))
	}
}

impl<V: Column> Operand<V> for V {
	fn into_value(self) -> Result<Value, EncodeError> {
		self.to_value()
	}
}

impl<V: Column> Operand<V> for &V {
	fn into_value(self) -> Result<Value, EncodeError> {
		self.to_value()
	}
}

impl<V: Column> Operand<Option<V>> for V {
	fn into_value(self) -> Result<Value, EncodeError> {
		self.to_value()
	}
}

impl<V: Column> Operand<Option<V>> for &V {
	fn into_value(self) -> Result<Value, EncodeError> {
		self.to_value()
	}
}

impl Operand<String> for &str {
	fn into_value(self) -> Result<Value, EncodeError> {
		Ok(Value::Text(self.to_string()))
	}
}

impl Operand<Option<String>> for &str {
	fn into_value(self) -> Result<Value, EncodeError> {
		Ok(Value::Text(self.to_string()))
	}
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
	Equal,
	NotEqual,
	Greater,
	GreaterOrEqual,
	Less,
	LessOrEqual,
}

impl Comparison {
	fn operator(self) -> &'static str {
		match self {
			Self::Equal => " = ",
			Self::NotEqual => " <> ",
			Self::Greater => " > ",
			Self::GreaterOrEqual => " >= ",
			Self::Less => " < ",
			Self::LessOrEqual => " <= ",
		}
	}
}

/// A condition on a table's rows. Combine with `&`, `|`, and `!`.
#[derive(Clone, Debug, PartialEq)]
#[must_use = "a condition does nothing until a query filters by it"]
pub enum Expr {
	#[doc(hidden)]
	Compare {
		column: &'static str,
		comparison: Comparison,
		value: Value,
	},
	/// A column compared with a column of another row: the row a policy
	/// checks, from inside one of its counts.
	#[doc(hidden)]
	CompareColumn {
		column: &'static str,
		comparison: Comparison,
		table: TableName,
		other: &'static str,
	},
	/// A column compared with a setting of the request: its bound value, or
	/// `current_setting(…)` in a row-level security policy.
	#[doc(hidden)]
	CompareSetting {
		column: &'static str,
		comparison: Comparison,
		setting: crate::settings::SettingRef,
	},
	#[doc(hidden)]
	In {
		column: &'static str,
		values: Vec<Value>,
	},
	#[doc(hidden)]
	Null { column: &'static str, negated: bool },
	#[doc(hidden)]
	Contains {
		column: &'static str,
		needle: String,
	},
	#[doc(hidden)]
	Like {
		column: &'static str,
		pattern: String,
	},
	#[doc(hidden)]
	And(Box<Expr>, Box<Expr>),
	#[doc(hidden)]
	Or(Box<Expr>, Box<Expr>),
	#[doc(hidden)]
	Not(Box<Expr>),
	/// How many rows of another query there are, compared with a number.
	#[doc(hidden)]
	Count {
		count: Box<crate::Count>,
		comparison: Comparison,
		value: i64,
	},
	/// The column is one of the values another table's rows hold: the rows
	/// of a link table, or the rows a request may see.
	#[doc(hidden)]
	Within {
		column: &'static str,
		selected: &'static str,
		rows: Box<crate::policy::Subquery>,
	},
	/// Every row, or none.
	#[doc(hidden)]
	Constant(bool),
	/// A value that couldn't be encoded; the query fails when it runs.
	#[doc(hidden)]
	Invalid(String),
}

impl Expr {
	/// A condition every row meets.
	pub const fn always() -> Self {
		Self::Constant(true)
	}

	/// A condition no row meets, such as a policy's rule for a request
	/// without the global it reads.
	pub const fn never() -> Self {
		Self::Constant(false)
	}
}

impl BitAnd for Expr {
	type Output = Self;

	fn bitand(self, other: Self) -> Self {
		Self::And(Box::new(self), Box::new(other))
	}
}

impl BitOr for Expr {
	type Output = Self;

	fn bitor(self, other: Self) -> Self {
		Self::Or(Box::new(self), Box::new(other))
	}
}

impl Not for Expr {
	type Output = Self;

	fn not(self) -> Self {
		Self::Not(Box::new(self))
	}
}

impl Expr {
	pub(crate) fn write(&self, sql: &mut Sql) -> Result<(), DbError> {
		match self {
			Self::Compare {
				column,
				comparison,
				value: Value::Null,
			} => {
				let negated = match comparison {
					Comparison::Equal => false,
					Comparison::NotEqual => true,
					_ => return Err(invalid("only `eq` and `ne` compare with `None`")),
				};
				Self::Null { column, negated }.write(sql)?;
			}
			Self::Compare {
				column,
				comparison,
				value,
			} => {
				sql.identifier(column)
					.push(comparison.operator())
					.value(value.clone());
			}
			Self::CompareColumn {
				column,
				comparison,
				table,
				other,
			} => {
				sql.identifier(column).push(comparison.operator());
				sql.outer_column(table, other)?;
			}
			Self::CompareSetting {
				column,
				comparison,
				setting,
			} => {
				sql.identifier(column).push(comparison.operator());

				if sql.renders_policy() {
					sql.push(&format!(
						"NULLIF(current_setting({}, true), '')::{}",
						leptoskit_db_schema::quote_literal(&format!(
							"{}{}",
							leptoskit_db_schema::SETTING_PREFIX,
							setting.name
						)),
						leptoskit_db_schema::postgres_type(&setting.ty)
					));
				} else {
					match &setting.value {
						Some(value) => {
							sql.value(value.clone());
						}
						None => {
							sql.push("NULL");
						}
					}
				}
			}
			Self::In { values, .. } if values.is_empty() => {
				sql.push("1 = 0");
			}
			Self::In { column, values } => {
				sql.identifier(column).push(" IN (");

				for (index, value) in values.iter().enumerate() {
					if index > 0 {
						sql.push(", ");
					}

					sql.value(value.clone());
				}

				sql.push(")");
			}
			Self::Null { column, negated } => {
				sql.identifier(column)
					.push(if *negated { " IS NOT NULL" } else { " IS NULL" });
			}
			Self::Contains { column, needle } => {
				let function = match sql.dialect() {
					Dialect::Postgres => "strpos(",
					Dialect::Sqlite => "instr(",
				};
				sql.push(function)
					.identifier(column)
					.push(", ")
					.value(Value::Text(needle.clone()))
					.push(") > 0");
			}
			Self::Like { column, pattern } => {
				let operator = match sql.dialect() {
					Dialect::Postgres => " ILIKE ",
					Dialect::Sqlite => " LIKE ",
				};
				sql.identifier(column)
					.push(operator)
					.value(Value::Text(pattern.clone()));
			}
			Self::And(left, right) => Self::binary(sql, left, " AND ", right)?,
			Self::Or(left, right) => Self::binary(sql, left, " OR ", right)?,
			Self::Not(inner) => {
				sql.push("NOT (");
				inner.write(sql)?;
				sql.push(")");
			}
			Self::Count {
				count,
				comparison,
				value,
			} => {
				count.write(sql)?;
				sql.push(comparison.operator()).value(Value::Int(*value));
			}
			Self::Within {
				column,
				selected,
				rows,
			} => {
				sql.identifier(column).push(" IN ");
				rows.write(sql, &quote_identifier(selected), "in")?;
			}
			Self::Constant(true) => {
				sql.push("1 = 1");
			}
			Self::Constant(false) => {
				sql.push("1 = 0");
			}
			Self::Invalid(message) => return Err(invalid(message)),
		}

		Ok(())
	}

	fn binary(sql: &mut Sql, left: &Self, operator: &str, right: &Self) -> Result<(), DbError> {
		sql.push("(");
		left.write(sql)?;
		sql.push(operator);
		right.write(sql)?;
		sql.push(")");
		Ok(())
	}
}

pub(crate) fn invalid(message: &str) -> DbError {
	DbError::Encode(EncodeError::new(message))
}

/// An ordering by one column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Order {
	column: &'static str,
	descending: bool,
}

impl Order {
	/// An ordering by a column known only by name, such as one the studio
	/// was asked to sort by.
	#[cfg(all(
		feature = "studio",
		debug_assertions,
		any(feature = "postgres", feature = "sqlite")
	))]
	pub(crate) const fn by(column: &'static str, descending: bool) -> Self {
		Self { column, descending }
	}

	pub(crate) fn write(self, sql: &mut Sql) {
		sql.identifier(self.column)
			.push(if self.descending { " DESC" } else { " ASC" });
	}
}

/// The unique columns an upsert conflicts on: one column, or a tuple.
pub trait ConflictTarget {
	#[doc(hidden)]
	fn columns(&self) -> Vec<&'static str>;
}

impl<T, V> ConflictTarget for Col<T, V> {
	fn columns(&self) -> Vec<&'static str> {
		vec![self.name]
	}
}

macro_rules! conflict_tuples {
	($(($($name:ident: $types:ident),+)),+ $(,)?) => {
		$(impl<T, $($types),+> ConflictTarget for ($(Col<T, $types>,)+) {
			fn columns(&self) -> Vec<&'static str> {
				let ($($name,)+) = self;
				vec![$($name.name),+]
			}
		})+
	};
}

conflict_tuples!((a: A, b: B), (a: A, b: B, c: C), (a: A, b: B, c: C, d: D));
