//! Live changes between servers: each transaction's changes as `NOTIFY`
//! payloads, small enough for Postgres, and back.

use std::collections::HashMap;

use leptoskit_db_schema::TableName;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use super::RowChange;
use crate::AuditOp;
use crate::Result;
use crate::Value;

/// The most bytes of a change set one notification carries: Postgres
/// refuses payloads of 8000 bytes or more, and each carries a header.
const CHUNK: usize = 7000;

/// One transaction's changes as `NOTIFY` payloads, each
/// `<id>:<index>:<count>:<part of the JSON>`, small enough for Postgres. A
/// transaction's notifications arrive together and in order, and the id
/// keeps those of different transactions apart.
pub(crate) fn encode(changes: &[RowChange]) -> Result<Vec<String>> {
	let wire: Vec<WireChange> = changes.iter().map(WireChange::from).collect();
	let json = serde_json::to_string(&wire)
		.map_err(|error| crate::expr::invalid(&format!("a live change didn't encode: {error}")))?;
	let mut parts = Vec::new();
	let mut rest = json.as_str();

	while !rest.is_empty() {
		let mut end = rest.len().min(CHUNK);

		while !rest.is_char_boundary(end) {
			end -= 1;
		}

		parts.push(&rest[..end]);
		rest = &rest[end..];
	}

	let id = Uuid::now_v7();
	let count = parts.len();
	Ok(parts
		.into_iter()
		.enumerate()
		.map(|(index, part)| format!("{id}:{index}:{count}:{part}"))
		.collect())
}

/// Puts change sets back together from their notifications.
#[derive(Default)]
pub(crate) struct Assembler {
	partial: HashMap<Uuid, Vec<String>>,
}

/// A notification that isn't a part of a change set this build reads.
#[derive(Debug, thiserror::Error)]
#[error("a live change arrived malformed: {0}")]
pub(crate) struct Malformed(String);

impl Assembler {
	/// Takes one notification, and returns the change set it completes.
	pub(crate) fn push(&mut self, payload: &str) -> Result<Option<Vec<RowChange>>, Malformed> {
		let malformed = || Malformed(payload.chars().take(80).collect());
		let mut fields = payload.splitn(4, ':');
		let (Some(id), Some(index), Some(count), Some(part)) =
			(fields.next(), fields.next(), fields.next(), fields.next())
		else {
			return Err(malformed());
		};
		let id: Uuid = id.parse().map_err(|_| malformed())?;
		let index: usize = index.parse().map_err(|_| malformed())?;
		let count: usize = count.parse().map_err(|_| malformed())?;
		let parts = self.partial.entry(id).or_default();

		if index != parts.len() || index >= count {
			self.partial.remove(&id);
			return Err(malformed());
		}

		parts.push(part.to_string());

		if parts.len() < count {
			return Ok(None);
		}

		let json = self.partial.remove(&id).unwrap_or_default().concat();
		let wire: Vec<WireChange> = serde_json::from_str(&json).map_err(|_| malformed())?;
		Ok(Some(wire.into_iter().map(RowChange::from).collect()))
	}

	/// Forgets change sets still missing parts, which a lost connection
	/// will never complete.
	pub(crate) fn clear(&mut self) {
		self.partial.clear();
	}
}

/// A [`RowChange`] as JSON, which keeps every value's type.
#[derive(Serialize, Deserialize)]
struct WireChange {
	#[serde(rename = "n", default, skip_serializing_if = "Option::is_none")]
	namespace: Option<String>,
	#[serde(rename = "t")]
	table: String,
	#[serde(rename = "o")]
	op: WireOp,
	#[serde(rename = "k")]
	key: Vec<WireValue>,
	#[serde(rename = "b", default, skip_serializing_if = "Option::is_none")]
	before: Option<Vec<WireValue>>,
	#[serde(rename = "a", default, skip_serializing_if = "Option::is_none")]
	after: Option<Vec<WireValue>>,
}

#[derive(Serialize, Deserialize)]
enum WireOp {
	#[serde(rename = "i")]
	Insert,
	#[serde(rename = "u")]
	Update,
	#[serde(rename = "d")]
	Delete,
}

/// A [`Value`], tagged with its kind. Floats travel as text, which keeps
/// `NaN` and the infinities, and reads back exactly.
#[derive(Serialize, Deserialize)]
enum WireValue {
	#[serde(rename = "z")]
	Null,
	#[serde(rename = "b")]
	Bool(bool),
	#[serde(rename = "i")]
	Int(i64),
	#[serde(rename = "f")]
	Float(String),
	#[serde(rename = "t")]
	Text(String),
	#[serde(rename = "x")]
	Bytes(Vec<u8>),
	#[serde(rename = "u")]
	Uuid(Uuid),
	#[serde(rename = "s")]
	Timestamp(chrono::DateTime<chrono::Utc>),
	#[serde(rename = "d")]
	Date(chrono::NaiveDate),
	#[serde(rename = "j")]
	Json(serde_json::Value),
}

impl From<&Value> for WireValue {
	fn from(value: &Value) -> Self {
		match value {
			Value::Null => Self::Null,
			Value::Bool(value) => Self::Bool(*value),
			Value::Int(value) => Self::Int(*value),
			Value::Float(value) => Self::Float(value.to_string()),
			Value::Text(value) => Self::Text(value.clone()),
			Value::Bytes(value) => Self::Bytes(value.clone()),
			Value::Uuid(value) => Self::Uuid(*value),
			Value::Timestamp(value) => Self::Timestamp(*value),
			Value::Date(value) => Self::Date(*value),
			Value::Json(value) => Self::Json(value.clone()),
		}
	}
}

impl From<WireValue> for Value {
	fn from(value: WireValue) -> Self {
		match value {
			WireValue::Null => Self::Null,
			WireValue::Bool(value) => Self::Bool(value),
			WireValue::Int(value) => Self::Int(value),
			// Every float wrote itself as text that parses back; anything
			// else isn't a float and matches no policy.
			WireValue::Float(value) => value.parse().map_or(Self::Null, Self::Float),
			WireValue::Text(value) => Self::Text(value),
			WireValue::Bytes(value) => Self::Bytes(value),
			WireValue::Uuid(value) => Self::Uuid(value),
			WireValue::Timestamp(value) => Self::Timestamp(value),
			WireValue::Date(value) => Self::Date(value),
			WireValue::Json(value) => Self::Json(value),
		}
	}
}

fn wire_values(values: &[Value]) -> Vec<WireValue> {
	values.iter().map(WireValue::from).collect()
}

fn values(values: Vec<WireValue>) -> Vec<Value> {
	values.into_iter().map(Value::from).collect()
}

impl From<&RowChange> for WireChange {
	fn from(change: &RowChange) -> Self {
		Self {
			namespace: change.table.namespace.clone(),
			table: change.table.name.clone(),
			op: match change.op {
				AuditOp::Insert => WireOp::Insert,
				AuditOp::Update => WireOp::Update,
				AuditOp::Delete => WireOp::Delete,
			},
			key: wire_values(&change.key),
			before: change.before.as_deref().map(wire_values),
			after: change.after.as_deref().map(wire_values),
		}
	}
}

impl From<WireChange> for RowChange {
	fn from(change: WireChange) -> Self {
		Self {
			table: TableName {
				namespace: change.namespace,
				name: change.table,
			},
			op: match change.op {
				WireOp::Insert => AuditOp::Insert,
				WireOp::Update => AuditOp::Update,
				WireOp::Delete => AuditOp::Delete,
			},
			key: values(change.key),
			before: change.before.map(values),
			after: change.after.map(values),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn change(op: AuditOp, text: String) -> RowChange {
		RowChange {
			table: TableName {
				namespace: Some("billing".into()),
				name: "invoices".into(),
			},
			op,
			key: vec![Value::Uuid(Uuid::from_u128(7))],
			before: Some(vec![
				Value::Null,
				Value::Bool(true),
				Value::Int(-3),
				Value::Float(f64::NAN),
				Value::Float(0.1),
			]),
			after: Some(vec![
				Value::Text(text),
				Value::Bytes(vec![0, 255]),
				Value::Timestamp(
					chrono::DateTime::from_timestamp(1_800_000_000, 123_456_789).unwrap(),
				),
				Value::Date(chrono::NaiveDate::from_ymd_opt(2026, 10, 30).unwrap()),
				Value::Json(serde_json::json!({ "a": [1, "b"] })),
			]),
		}
	}

	fn same(left: &RowChange, right: &RowChange) -> bool {
		// `NaN` is never equal to itself, so compare the debug text.
		format!("{left:?}") == format!("{right:?}")
	}

	#[test]
	fn change_sets_survive_notifications_of_any_size() {
		// Multi-byte text across chunk boundaries, in one and many parts.
		for text in ["small".to_string(), "é🙂".repeat(5000)] {
			let changes = vec![
				change(AuditOp::Insert, text.clone()),
				change(AuditOp::Update, text.clone()),
				change(AuditOp::Delete, text),
			];
			let payloads = encode(&changes).unwrap();
			assert!(payloads.iter().all(|payload| payload.len() < 8000));
			let mut assembler = Assembler::default();
			let mut done = None;

			for (index, payload) in payloads.iter().enumerate() {
				let result = assembler.push(payload).unwrap();
				assert_eq!(result.is_some(), index + 1 == payloads.len());
				done = result;
			}

			let done = done.unwrap();
			assert_eq!(done.len(), changes.len());
			assert!(
				done.iter()
					.zip(&changes)
					.all(|(left, right)| same(left, right))
			);
		}
	}

	#[test]
	fn malformed_and_out_of_order_notifications_are_refused() {
		let payloads = encode(&[change(AuditOp::Insert, "é".repeat(8000))]).unwrap();
		assert!(payloads.len() > 1);

		let mut assembler = Assembler::default();
		assert!(assembler.push(&payloads[1]).is_err());
		assert!(assembler.push(&payloads[0]).unwrap().is_none());
		assembler.clear();
		assert!(assembler.push(&payloads[1]).is_err());

		for payload in [
			"",
			"x:0:1:[]",
			"00000000-0000-0000-0000-000000000000:0:x:[]",
		] {
			assert!(Assembler::default().push(payload).is_err(), "{payload}");
		}

		let id = Uuid::nil();
		assert!(
			Assembler::default()
				.push(&format!("{id}:0:1:not json"))
				.is_err()
		);
		assert!(Assembler::default().push(&format!("{id}:1:1:[]")).is_err());
		assert_eq!(
			Assembler::default().push(&format!("{id}:0:1:[]")).unwrap(),
			Some(Vec::new())
		);
		assert_eq!(
			Value::from(WireValue::Float("not a float".into())),
			Value::Null
		);
	}
}
