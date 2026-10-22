//! Editor schema generated from the deserialized configuration types.

use schemars::Schema;
use serde_json::Value;

use crate::FNS_SCAFFOLDS;
use crate::FNS_TARGETS;
use crate::LeptoskitConfig;
use crate::SCHEMA_VERSION;

/// Generate the current JSON Schema (draft 2020-12) with a public asset ID.
pub fn configuration(id: &str) -> Schema {
	let mut schema = schemars::generate::SchemaSettings::draft2020_12()
		.for_deserialize()
		.into_generator()
		.into_root_schema_for::<LeptoskitConfig>();
	schema.insert("$id".into(), Value::String(id.into()));
	schema.insert("title".into(), Value::String("leptoskit.toml".into()));
	// `validate` also requires a registry and trusted key when code push is
	// enabled. Mirror that conditional in the editor contract.
	let definitions = schema
		.ensure_object()
		.get_mut("$defs")
		.expect("configuration references nested definitions");
	definitions["CodePushConfig"]["allOf"] = serde_json::json!([{
		"if": { "properties": { "enabled": { "const": true } }, "required": ["enabled"] },
		"then": {
			"required": ["server", "public_keys"],
			"properties": {
				"server": { "pattern": "\\S" },
				"public_keys": { "minItems": 1 }
			}
		}
	}]);
	let target = &mut definitions["FnsTarget"];
	target["required"] = serde_json::json!(["language"]);
	target["properties"]["language"]["enum"] = serde_json::json!(FNS_TARGETS);
	target["properties"]["language"]
		.as_object_mut()
		.expect("client language is a string schema")
		.remove("default");
	let scaffolds: Vec<Option<&str>> = FNS_SCAFFOLDS
		.iter()
		.copied()
		.map(Some)
		.chain(std::iter::once(None))
		.collect();
	target["properties"]["scaffold"]["enum"] = serde_json::json!(scaffolds);
	schema
}

pub(crate) fn public_keys(_: &mut schemars::SchemaGenerator) -> Schema {
	schemars::json_schema!({
		"type": "array",
		"items": { "type": "string", "pattern": "^[0-9a-fA-F]{64}$" }
	})
}

pub(crate) fn version(_: &mut schemars::SchemaGenerator) -> Schema {
	schemars::json_schema!({ "type": "integer", "const": SCHEMA_VERSION })
}
