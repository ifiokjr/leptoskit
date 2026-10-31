//! Recording live changes as writes make them.

use leptoskit_db_schema::TableName;

use crate::AuditOp;
use crate::Value;
use crate::audit::Subject;

/// One row's change, as live subscribers learn of it: the row before and
/// after, in the table's column order, with its `secret` columns `NULL`.
/// The images only ever decide who hears of the change; subscribers get
/// the table and the key.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub struct RowChange {
	pub(crate) table: TableName,
	pub(crate) op: AuditOp,
	pub(crate) key: Vec<Value>,
	pub(crate) before: Option<Vec<Value>>,
	pub(crate) after: Option<Vec<Value>>,
}

impl RowChange {
	/// A change to a row of `subject`, from its normalized values.
	pub(crate) fn new(
		subject: &Subject,
		table: TableName,
		op: AuditOp,
		before: Option<&[Value]>,
		after: Option<&[Value]>,
	) -> Self {
		let image = |values: &[Value]| -> Vec<Value> {
			subject
				.columns
				.iter()
				.zip(values)
				.map(|(column, value)| {
					if subject.secret.contains(column) {
						Value::Null
					} else {
						value.clone()
					}
				})
				.collect()
		};
		let key = after
			.or(before)
			.map(|values| subject.key_of(values))
			.unwrap_or_default();

		Self {
			table,
			op,
			key,
			before: before.map(image),
			after: after.map(image),
		}
	}
}
