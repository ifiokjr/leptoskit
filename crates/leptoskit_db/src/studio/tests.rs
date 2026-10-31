//! What the studio reads from text typed into it, and shows of values.

use leptoskit_db_schema::ColumnType;

use super::ops::Cell;
use super::ops::Op;
use super::ops::display;
use super::ops::parse_value;
use crate::Value;

#[test]
fn typed_text_becomes_a_value_of_the_columns_type() {
	let parse = |ty: ColumnType, text: &str| parse_value(&ty, text);

	assert_eq!(parse(ColumnType::Bool, " true "), Ok(Value::Bool(true)));
	assert_eq!(parse(ColumnType::Bool, "false"), Ok(Value::Bool(false)));
	assert_eq!(parse(ColumnType::Int16, "-7"), Ok(Value::Int(-7)));
	assert_eq!(
		parse(ColumnType::Int16, "40000"),
		Err("40000 is outside -32768 to 32767".into())
	);
	assert!(
		parse(ColumnType::Int32, "x")
			.unwrap_err()
			.contains("whole number")
	);
	assert_eq!(parse(ColumnType::Int64, "9"), Ok(Value::Int(9)));
	assert_eq!(parse(ColumnType::Float64, "1.5"), Ok(Value::Float(1.5)));
	assert!(
		parse(ColumnType::Float32, "one")
			.unwrap_err()
			.contains("number")
	);
	assert_eq!(
		parse(ColumnType::TEXT, " as typed "),
		Ok(Value::Text(" as typed ".into()))
	);

	let uuid = uuid::Uuid::now_v7();
	assert_eq!(
		parse(ColumnType::Uuid, &uuid.to_string()),
		Ok(Value::Uuid(uuid))
	);
	assert!(parse(ColumnType::Uuid, "ada").unwrap_err().contains("UUID"));
	assert!(matches!(
		parse(ColumnType::Timestamp, "2026-10-30T12:00:00+01:00"),
		Ok(Value::Timestamp(instant)) if instant.to_rfc3339() == "2026-10-30T11:00:00+00:00"
	));
	assert!(
		parse(ColumnType::Timestamp, "noon")
			.unwrap_err()
			.contains("RFC 3339")
	);
	assert_eq!(
		parse(ColumnType::Date, "2026-10-30"),
		Ok(Value::Date("2026-10-30".parse().unwrap()))
	);
	assert!(
		parse(ColumnType::Date, "today")
			.unwrap_err()
			.contains("date")
	);
	assert_eq!(
		parse(ColumnType::Json, r#"{"a":1}"#),
		Ok(Value::Json(serde_json::json!({ "a": 1 })))
	);
	assert!(parse(ColumnType::Json, "{").unwrap_err().contains("JSON"));
	assert!(
		parse(ColumnType::Bytes, "00")
			.unwrap_err()
			.contains("can't read or write")
	);
}

#[test]
fn values_show_as_text() {
	let uuid = uuid::Uuid::now_v7();
	assert_eq!(display(&Value::Null), Cell::Null);
	assert_eq!(display(&Value::Null).text(), "NULL");
	assert_eq!(Cell::Secret.text(), "‹secret›");
	assert_eq!(display(&Value::Bool(true)).text(), "true");
	assert_eq!(display(&Value::Int(3)).text(), "3");
	assert_eq!(display(&Value::Float(0.5)).text(), "0.5");
	assert_eq!(display(&Value::Uuid(uuid)).text(), uuid.to_string());
	// SQLite stores UUIDs as 16 bytes.
	assert_eq!(
		display(&Value::Bytes(uuid.as_bytes().to_vec())).text(),
		uuid.to_string()
	);
	assert_eq!(display(&Value::Bytes(vec![1, 2, 3])).text(), "3 bytes");
	assert_eq!(
		display(&Value::Date("2026-10-30".parse().unwrap())).text(),
		"2026-10-30"
	);
	assert_eq!(display(&Value::Json(serde_json::json!([1]))).text(), "[1]");
}

#[test]
fn filters_name_their_comparisons() {
	for op in Op::ALL {
		assert_eq!(Op::parse(op.name()), Some(op));
		assert_ne!(op.label(), "");
	}
	assert_eq!(Op::parse("like"), None);
}

#[test]
fn tokens_are_compared_whole() {
	assert!(super::same("abc", "abc"));
	assert!(!super::same("abc", "abd"));
	assert!(!super::same("abc", "abcd"));
	let token = super::token().unwrap();
	assert_eq!(token.len(), 64);
	assert_ne!(token, super::token().unwrap());
}
