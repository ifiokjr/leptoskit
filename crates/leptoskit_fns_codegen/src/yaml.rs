//! A small JSON-to-YAML writer for the OpenAPI document.
//!
//! The YAML ecosystem's `serde` crates are unmaintained, and an OpenAPI
//! document needs only a fraction of YAML: nested maps, sequences, and
//! scalars. Every string — key or value — is written as a double-quoted
//! scalar using JSON escaping, which YAML 1.2 accepts verbatim, so no scalar
//! can ever be misread as a number, boolean, or null.

use serde_json::Map;
use serde_json::Value;

/// Renders a JSON value as a YAML document.
#[must_use]
pub fn to_yaml(value: &Value) -> String {
	let mut out = String::new();
	match value {
		Value::Object(map) if !map.is_empty() => write_object(map, "", "", &mut out),
		Value::Array(items) if !items.is_empty() => write_array(items, "", &mut out),
		scalar => {
			out.push_str(&scalar_text(scalar));
			out.push('\n');
		}
	}
	out
}

/// Writes a map whose first entry starts with `first` and whose remaining
/// entries start with `rest`: identical for a plain nested map, and `- ` vs.
/// two spaces for a map that is a sequence item.
fn write_object(map: &Map<String, Value>, first: &str, rest: &str, out: &mut String) {
	for (index, (key, value)) in map.iter().enumerate() {
		out.push_str(if index == 0 { first } else { rest });
		out.push_str(&scalar_text(&Value::String(key.clone())));
		out.push(':');
		write_nested(value, rest, out);
	}
}

/// Writes a value that follows a `key:` or a `- ` on the current line.
fn write_nested(value: &Value, indent: &str, out: &mut String) {
	let child = format!("{indent}  ");
	match value {
		Value::Object(map) if !map.is_empty() => {
			out.push('\n');
			write_object(map, &child, &child, out);
		}
		Value::Array(items) if !items.is_empty() => {
			out.push('\n');
			write_array(items, &child, out);
		}
		scalar => {
			out.push(' ');
			out.push_str(&scalar_text(scalar));
			out.push('\n');
		}
	}
}

fn write_array(items: &[Value], indent: &str, out: &mut String) {
	let dash = format!("{indent}- ");
	let continuation = format!("{indent}  ");
	for item in items {
		match item {
			Value::Object(map) if !map.is_empty() => {
				write_object(map, &dash, &continuation, out);
			}
			Value::Array(nested) if !nested.is_empty() => {
				out.push_str(indent);
				out.push_str("-\n");
				write_array(nested, &continuation, out);
			}
			scalar => {
				out.push_str(&dash);
				out.push_str(&scalar_text(scalar));
				out.push('\n');
			}
		}
	}
}

/// A scalar (or empty container) in flow form.
fn scalar_text(value: &Value) -> String {
	match value {
		Value::Null => "null".to_owned(),
		Value::Bool(flag) => flag.to_string(),
		Value::Number(number) => number.to_string(),
		// JSON strings are valid YAML double-quoted scalars.
		Value::String(_) => value.to_string(),
		Value::Array(_) => "[]".to_owned(),
		Value::Object(_) => "{}".to_owned(),
	}
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	#[test]
	fn nests_maps_sequences_and_scalars() {
		let value = json!({
			"openapi": "3.1.0",
			"info": { "title": "demo", "version": "0.1.0" },
			"servers": [{ "url": "https://a", "x": 1 }, { "url": "https://b" }],
			"tags": ["todos", "users"],
			"empty": {},
			"none": [],
			"flag": true,
			"nothing": null,
			"nested": [["a", "b"], ["c"]]
		});
		let yaml = to_yaml(&value);
		// serde_json sorts keys, so the output order is alphabetical.
		let expected = r#""empty": {}
"flag": true
"info":
  "title": "demo"
  "version": "0.1.0"
"nested":
  -
    - "a"
    - "b"
  -
    - "c"
"none": []
"nothing": null
"openapi": "3.1.0"
"servers":
  - "url": "https://a"
    "x": 1
  - "url": "https://b"
"tags":
  - "todos"
  - "users"
"#;
		assert_eq!(yaml, expected);
	}

	#[test]
	fn top_level_values_of_every_kind_render() {
		assert_eq!(to_yaml(&json!(["a", 1])), "- \"a\"\n- 1\n");
		assert_eq!(to_yaml(&json!(true)), "true\n");
		assert_eq!(to_yaml(&json!({})), "{}\n");
		assert_eq!(to_yaml(&json!([])), "[]\n");
		assert_eq!(to_yaml(&json!(null)), "null\n");
		assert_eq!(to_yaml(&json!([{ "a": [1] }])), "- \"a\":\n    - 1\n");
	}

	#[test]
	fn strings_that_look_like_other_types_stay_strings() {
		let yaml = to_yaml(&json!({ "true": "3", "null": "yes: no" }));
		assert!(yaml.contains("\"true\": \"3\""));
		assert!(yaml.contains("\"null\": \"yes: no\""));
	}
}
