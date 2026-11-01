//! Deciding without the database whether a row could meet a condition: how
//! a live query skips the writes that can't change its result.
//!
//! The rule is soundness: [`Expr::eval`] says [`Tri::False`] only when both
//! databases would certainly not match the row. Anything that depends on the
//! dialect, the collation, or another table is [`Tri::Unknown`], which costs a
//! re-run at worst.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::hash::BuildHasher;

use chrono::Datelike;
use chrono::Timelike;

use crate::Expr;
use crate::Value;
use crate::expr::Comparison;

/// Whether a row meets a condition, in SQL's three-valued logic: true,
/// false, or unknown. Unknown is what SQL calls `NULL`, and also "can't tell
/// without the database".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tri {
	/// The row meets the condition.
	True,
	/// The row certainly doesn't.
	False,
	/// It may or may not: SQL's `NULL`, or a condition only the database can
	/// decide.
	Unknown,
}

impl Tri {
	/// Both: false if either is, else unknown if either is.
	#[must_use]
	pub const fn and(self, other: Self) -> Self {
		match (self, other) {
			(Self::False, _) | (_, Self::False) => Self::False,
			(Self::True, Self::True) => Self::True,
			_ => Self::Unknown,
		}
	}

	/// Either: true if either is, else unknown if either is.
	#[must_use]
	pub const fn or(self, other: Self) -> Self {
		match (self, other) {
			(Self::True, _) | (_, Self::True) => Self::True,
			(Self::False, Self::False) => Self::False,
			_ => Self::Unknown,
		}
	}

	/// Whether the row may meet the condition: true or unknown. Only a row
	/// that can't is safe to ignore.
	pub const fn may_match(self) -> bool {
		!matches!(self, Self::False)
	}
}

impl std::ops::Not for Tri {
	type Output = Self;

	fn not(self) -> Self {
		match self {
			Self::True => Self::False,
			Self::False => Self::True,
			Self::Unknown => Self::Unknown,
		}
	}
}

impl From<bool> for Tri {
	fn from(value: bool) -> Self {
		if value { Self::True } else { Self::False }
	}
}

/// A row to evaluate a condition on: its values, by column name.
///
/// Implemented for maps from column names to values. A row that doesn't hold
/// a column makes every condition on it unknown.
pub trait Row {
	/// The value of `column`, or `None` when the row doesn't hold it.
	fn value(&self, column: &str) -> Option<Cow<'_, Value>>;
}

impl<R: Row + ?Sized> Row for &R {
	fn value(&self, column: &str) -> Option<Cow<'_, Value>> {
		(**self).value(column)
	}
}

impl<S: BuildHasher> Row for HashMap<String, Value, S> {
	fn value(&self, column: &str) -> Option<Cow<'_, Value>> {
		self.get(column).map(Cow::Borrowed)
	}
}

impl Row for BTreeMap<String, Value> {
	fn value(&self, column: &str) -> Option<Cow<'_, Value>> {
		self.get(column).map(Cow::Borrowed)
	}
}

impl Row for [(&str, Value)] {
	fn value(&self, column: &str) -> Option<Cow<'_, Value>> {
		self.iter()
			.find(|(name, _)| *name == column)
			.map(|(_, value)| Cow::Borrowed(value))
	}
}

impl Expr {
	/// Whether `row` meets the condition, as a query's `WHERE` on its table
	/// would decide, without asking the database.
	///
	/// It says [`Tri::False`] only when the database would certainly not
	/// match the row, so a live query can skip a write to it. Whatever the
	/// two dialects might decide differently is [`Tri::Unknown`]:
	///
	/// - `ilike` and `contains`, which fold case and match text differently;
	/// - ordering text, which follows the database's collation;
	/// - counts, `exists`, and the rows a link or a variant looks up, which
	///   read other rows;
	/// - a column of another row, or one `row` doesn't hold;
	/// - a request setting the request doesn't have;
	/// - geospatial conditions, which PostGIS and SQLite's layer compute
	///   differently, and a full-text search's match;
	/// - JSON, `NaN`, and instants finer than a millisecond or outside the
	///   years 0 to 9999, which the databases store differently.
	///
	/// ```ignore
	/// let condition = Message::columns().channel_id.eq(channel);
	/// let row = HashMap::from([("channel_id".to_string(), Value::Uuid(other.uuid()))]);
	/// assert_eq!(condition.eval(&row), Tri::False);
	/// ```
	pub fn eval<R: Row + ?Sized>(&self, row: &R) -> Tri {
		match self {
			Self::Compare {
				column,
				comparison,
				value,
			} => {
				let Some(found) = row.value(column) else {
					return Tri::Unknown;
				};

				match (comparison, value) {
					// `eq(None)` and `ne(None)` are `IS NULL` and `IS NOT
					// NULL`.
					(Comparison::Equal, Value::Null) => Tri::from(*found == Value::Null),
					(Comparison::NotEqual, Value::Null) => Tri::from(*found != Value::Null),
					// Any other comparison with `None` fails to run.
					(_, Value::Null) => Tri::Unknown,
					_ => compare(&found, *comparison, value),
				}
			}
			Self::CompareColumn {
				column,
				comparison,
				other,
				..
			} => {
				// At a query's top level, another column of its own table is
				// the same row's.
				match (row.value(column), row.value(other)) {
					(Some(left), Some(right)) => compare(&left, *comparison, &right),
					_ => Tri::Unknown,
				}
			}
			// A setting binds its value, or `NULL` without one, so it compares
			// as a value would, except that `NULL` never matches.
			Self::CompareSetting {
				column,
				comparison,
				setting,
			} => {
				match (row.value(column), &setting.value) {
					(Some(found), Some(value)) => compare(&found, *comparison, value),
					_ => Tri::Unknown,
				}
			}
			Self::In { values, .. } if values.is_empty() => Tri::False,
			Self::In { column, values } => {
				let Some(found) = row.value(column) else {
					return Tri::Unknown;
				};

				values.iter().fold(Tri::False, |any, value| {
					any.or(compare(&found, Comparison::Equal, value))
				})
			}
			Self::Null { column, negated } => {
				row.value(column).map_or(Tri::Unknown, |found| {
					Tri::from((*found == Value::Null) != *negated)
				})
			}
			Self::And(left, right) => left.eval(row).and(right.eval(row)),
			Self::Or(left, right) => left.eval(row).or(right.eval(row)),
			Self::Not(inner) => !inner.eval(row),
			Self::Constant(value) => Tri::from(*value),
			#[cfg(feature = "geo")]
			Self::Spatial(_) => Tri::Unknown,
			Self::InSetting { .. }
			| Self::Contains { .. }
			| Self::Like { .. }
			| Self::Count { .. }
			| Self::Within { .. }
			| Self::Correlated { .. }
			| Self::Opaque(_)
			| Self::Invalid(_) => Tri::Unknown,
			#[cfg(feature = "vector")]
			Self::Stale(_) => Tri::Unknown,
		}
	}
}

/// `left comparison right`, as both databases would decide it, or unknown
/// when either value is `NULL` or the databases might disagree.
fn compare(left: &Value, comparison: Comparison, right: &Value) -> Tri {
	let ordering = match (left, right) {
		(Value::Bool(left), Value::Bool(right)) => left.cmp(right),
		(Value::Int(left), Value::Int(right)) => left.cmp(right),
		(Value::Float(left), Value::Float(right)) => {
			return compare_floats(*left, comparison, *right);
		}
		(Value::Uuid(left), Value::Uuid(right)) => left.as_bytes().cmp(right.as_bytes()),
		(Value::Bytes(left), Value::Bytes(right)) => left.cmp(right),
		(Value::Timestamp(left), Value::Timestamp(right))
			if portable_instant(left) && portable_instant(right) =>
		{
			left.cmp(right)
		}
		(Value::Date(left), Value::Date(right))
			if portable_year(left.year()) && portable_year(right.year()) =>
		{
			left.cmp(right)
		}
		// Equal text is equal in every collation; anything else is ordered
		// by the database's.
		(Value::Text(left), Value::Text(right)) => {
			match comparison {
				_ if left == right => Ordering::Equal,
				Comparison::Equal => return Tri::False,
				Comparison::NotEqual => return Tri::True,
				_ => return Tri::Unknown,
			}
		}
		// `NULL`, JSON, which Postgres's `json` can't compare, and values of
		// different kinds, which one database would convert and the other
		// refuse.
		_ => return Tri::Unknown,
	};

	Tri::from(comparison.holds(ordering))
}

/// Floats compare as `f64`, except that Postgres binds a value compared with
/// a `real` column as an `f32`. The answer is certain only when both agree;
/// `NaN`, which SQLite stores as `NULL`, never is.
fn compare_floats(left: f64, comparison: Comparison, right: f64) -> Tri {
	let (Some(wide), Some(narrow)) = (
		left.partial_cmp(&right),
		(left as f32).partial_cmp(&(right as f32)),
	) else {
		return Tri::Unknown;
	};
	let holds = comparison.holds(wide);

	if holds == comparison.holds(narrow) {
		Tri::from(holds)
	} else {
		Tri::Unknown
	}
}

/// Whether both databases store the instant exactly: SQLite keeps
/// milliseconds, as fixed-width text that orders by time only between the
/// years 0 and 9999.
fn portable_instant(instant: &crate::Timestamp) -> bool {
	instant.nanosecond() % 1_000_000 == 0 && portable_year(instant.year())
}

/// Whether SQLite's text for a date in this year orders by time.
fn portable_year(year: i32) -> bool {
	(0..=9999).contains(&year)
}

#[cfg(test)]
mod tests {
	use chrono::NaiveDate;
	use chrono::TimeZone;
	use chrono::Utc;
	use uuid::Uuid;

	use super::*;

	fn compare_with(column: &'static str, comparison: Comparison, value: Value) -> Expr {
		Expr::Compare {
			column,
			comparison,
			value,
		}
	}

	fn row(values: &[(&'static str, Value)]) -> BTreeMap<String, Value> {
		values
			.iter()
			.map(|(name, value)| ((*name).to_string(), value.clone()))
			.collect()
	}

	#[test]
	fn logic_is_kleenes() {
		use Tri::False;
		use Tri::True;
		use Tri::Unknown;

		for (left, right, and, or) in [
			(True, True, True, True),
			(True, False, False, True),
			(True, Unknown, Unknown, True),
			(False, False, False, False),
			(False, Unknown, False, Unknown),
			(Unknown, Unknown, Unknown, Unknown),
		] {
			assert_eq!(left.and(right), and);
			assert_eq!(right.and(left), and);
			assert_eq!(left.or(right), or);
			assert_eq!(right.or(left), or);
		}

		assert_eq!(!True, False);
		assert_eq!(!False, True);
		assert_eq!(!Unknown, Unknown);
		assert!(Unknown.may_match() && True.may_match() && !False.may_match());
	}

	#[test]
	fn comparisons_follow_sql_null_semantics() {
		let row = row(&[("n", Value::Int(3)), ("missing", Value::Null)]);
		let gt = |value| compare_with("n", Comparison::Greater, value);

		assert_eq!(gt(Value::Int(2)).eval(&row), Tri::True);
		assert_eq!(gt(Value::Int(3)).eval(&row), Tri::False);
		assert_eq!(
			compare_with("missing", Comparison::Equal, Value::Int(1)).eval(&row),
			Tri::Unknown
		);
		assert_eq!(
			(!compare_with("missing", Comparison::Equal, Value::Int(1))).eval(&row),
			Tri::Unknown
		);
		// `eq(None)` is `IS NULL`.
		assert_eq!(
			compare_with("missing", Comparison::Equal, Value::Null).eval(&row),
			Tri::True
		);
		assert_eq!(
			compare_with("n", Comparison::NotEqual, Value::Null).eval(&row),
			Tri::True
		);
		assert_eq!(gt(Value::Null).eval(&row), Tri::Unknown);
		// A column the row doesn't hold.
		assert_eq!(
			compare_with("other", Comparison::Equal, Value::Int(1)).eval(&row),
			Tri::Unknown
		);
		assert_eq!(
			Expr::Null {
				column: "other",
				negated: false
			}
			.eval(&row),
			Tri::Unknown
		);
		assert_eq!(
			Expr::Null {
				column: "missing",
				negated: true
			}
			.eval(&row),
			Tri::False
		);
	}

	#[test]
	fn lists_are_any_equal() {
		let row = row(&[("n", Value::Int(3)), ("missing", Value::Null)]);
		let within = |column, values: Vec<Value>| Expr::In { column, values }.eval(&row);

		assert_eq!(within("n", vec![]), Tri::False);
		assert_eq!(within("missing", vec![]), Tri::False);
		assert_eq!(within("n", vec![Value::Int(1), Value::Int(3)]), Tri::True);
		assert_eq!(within("n", vec![Value::Int(1)]), Tri::False);
		assert_eq!(within("n", vec![Value::Int(1), Value::Null]), Tri::Unknown);
		assert_eq!(within("missing", vec![Value::Int(1)]), Tri::Unknown);
		assert_eq!(within("other", vec![Value::Int(1)]), Tri::Unknown);
	}

	#[test]
	fn text_is_ordered_only_by_the_database() {
		let row = row(&[("name", Value::Text("b".into()))]);
		let name = |comparison, text: &str| {
			compare_with("name", comparison, Value::Text(text.into())).eval(&row)
		};

		assert_eq!(name(Comparison::Equal, "b"), Tri::True);
		assert_eq!(name(Comparison::Equal, "B"), Tri::False);
		assert_eq!(name(Comparison::NotEqual, "B"), Tri::True);
		assert_eq!(name(Comparison::Less, "c"), Tri::Unknown);
		assert_eq!(name(Comparison::LessOrEqual, "b"), Tri::True);
		assert_eq!(name(Comparison::Greater, "b"), Tri::False);
	}

	#[test]
	fn values_the_databases_store_differently_are_unknown() {
		let at = |millis: i64| Utc.timestamp_millis_opt(millis).unwrap();
		let fine = at(1_000) + chrono::TimeDelta::microseconds(1);
		let ancient = NaiveDate::from_ymd_opt(-1, 1, 1).unwrap();
		let date = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
		let cases = [
			(
				Value::Timestamp(at(1_000)),
				Value::Timestamp(at(2_000)),
				Tri::True,
			),
			(
				Value::Timestamp(fine),
				Value::Timestamp(at(2_000)),
				Tri::Unknown,
			),
			(
				Value::Date(date),
				Value::Date(date.succ_opt().unwrap()),
				Tri::True,
			),
			(Value::Date(ancient), Value::Date(date), Tri::Unknown),
			(Value::Float(1.0), Value::Float(2.0), Tri::True),
			(Value::Float(f64::NAN), Value::Float(2.0), Tri::Unknown),
			// Equal as `f32`, which Postgres compares a `real` column as.
			(
				Value::Float(0.1),
				Value::Float(f64::from(0.1_f32)),
				Tri::Unknown,
			),
			(Value::Bool(false), Value::Bool(true), Tri::True),
			(Value::Int(1), Value::Float(2.0), Tri::Unknown),
			(
				Value::Json(serde_json::json!(1)),
				Value::Json(serde_json::json!(2)),
				Tri::Unknown,
			),
			(Value::Bytes(vec![0, 1]), Value::Bytes(vec![1]), Tri::True),
			(
				Value::Uuid(Uuid::from_u128(1)),
				Value::Uuid(Uuid::from_u128(1 << 64)),
				Tri::True,
			),
		];

		for (left, right, expected) in cases {
			let row = row(&[("left", left.clone()), ("right", right.clone())]);
			let less = Expr::CompareColumn {
				column: "left",
				comparison: Comparison::Less,
				table: leptoskit_db_schema::TableName {
					namespace: None,
					name: "rows".into(),
				},
				other: "right",
			};
			assert_eq!(less.eval(&row), expected, "{left:?} < {right:?}");
		}
	}

	#[test]
	fn other_rows_and_text_matching_are_unknown() {
		let row: &[(&str, Value)] = &[("name", Value::Text("ab".into()))];
		let table = leptoskit_db_schema::TableName {
			namespace: None,
			name: "rows".into(),
		};

		for condition in [
			Expr::Contains {
				column: "name",
				needle: "a".into(),
			},
			Expr::Like {
				column: "name",
				pattern: "z%".into(),
			},
			Expr::Correlated {
				column: "name",
				comparison: Comparison::Equal,
				table: table.clone(),
				other: "name",
			},
			Expr::CompareColumn {
				column: "name",
				comparison: Comparison::Equal,
				table,
				other: "missing",
			},
			Expr::Opaque("a full-text search's match"),
			Expr::Invalid("nope".into()),
		] {
			assert_eq!(condition.eval(row), Tri::Unknown, "{condition:?}");
			assert_eq!((!condition).eval(row), Tri::Unknown);
		}

		assert_eq!(Expr::always().eval(row), Tri::True);
		assert_eq!(
			(Expr::never() & Expr::Invalid(String::new())).eval(row),
			Tri::False
		);
		assert_eq!(
			(Expr::always() | Expr::Invalid(String::new())).eval(&row),
			Tri::True
		);
		let map: HashMap<String, Value> = HashMap::from([("name".into(), Value::Int(1))]);
		assert_eq!(
			compare_with("name", Comparison::Equal, Value::Int(1)).eval(&map),
			Tri::True
		);
	}

	#[test]
	fn settings_compare_as_their_value_and_never_match_without_one() {
		let row = row(&[("owner", Value::Int(7))]);
		let owner = |value: Option<Value>| {
			Expr::CompareSetting {
				column: "owner",
				comparison: Comparison::Equal,
				setting: crate::settings::SettingRef {
					name: "owner",
					ty: leptoskit_db_schema::ColumnType::Int64,
					value,
				},
			}
		};

		assert_eq!(owner(Some(Value::Int(7))).eval(&row), Tri::True);
		assert_eq!(owner(Some(Value::Int(8))).eval(&row), Tri::False);
		// `= NULL`, which no row meets, and neither does its negation.
		assert_eq!(owner(Some(Value::Null)).eval(&row), Tri::Unknown);
		assert_eq!(owner(None).eval(&row), Tri::Unknown);
		assert_eq!((!owner(None)).eval(&row), Tri::Unknown);
	}
}
