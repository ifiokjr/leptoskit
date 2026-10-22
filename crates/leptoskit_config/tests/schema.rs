//! Validate real TOML documents against the generated editor contract.
#![cfg(feature = "schema")]

use std::path::Path;

use leptoskit_config::LeptoskitConfig;
use leptoskit_config::schema;
use leptoskit_config::schema_url;
use serde_json::Value;

fn document(raw: &str) -> Value {
	let value: toml::Value = toml::from_str(raw).expect("valid TOML");
	serde_json::to_value(value).expect("TOML as JSON")
}

#[test]
fn validates_defaults_and_every_project_configuration() {
	let schema = serde_json::to_value(schema::configuration(&schema_url())).expect("schema");
	let validator = jsonschema::validator_for(&schema).expect("valid schema");
	let defaults = LeptoskitConfig::default()
		.to_toml_string()
		.expect("defaults");
	validator
		.validate(&document(&defaults))
		.expect("default configuration");
	validator
		.validate(&document(""))
		.expect("all keys are optional");
	validator
		.validate(&document("[router]\n[format]\n"))
		.expect("optional sections");
	validator
		.validate(&document(
			"[[fns.targets]]\nlanguage = 'typescript'\nscaffold = 'full'\n",
		))
		.expect("supported RPC client target");

	fn visit(path: &Path, validator: &jsonschema::Validator) -> usize {
		let mut count = 0;

		for entry in std::fs::read_dir(path).expect("read examples") {
			let path = entry.expect("entry").path();

			if path.is_dir() && path.file_name().is_some_and(|name| name != "target") {
				count += visit(&path, validator);
			} else if path
				.file_name()
				.is_some_and(|name| name == "leptoskit.toml")
			{
				let raw = std::fs::read_to_string(&path).expect("read config");
				validator
					.validate(&document(&raw))
					.unwrap_or_else(|error| panic!("{}: {error}", path.display()));
				count += 1;
			}
		}

		count
	}

	let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
	assert!(visit(&root, &validator) > 0, "exercised real project files");
}

#[test]
fn rejects_unknown_keys_enums_versions_and_out_of_range_numbers() {
	let schema = serde_json::to_value(schema::configuration(&schema_url())).expect("schema");
	let validator = jsonschema::validator_for(&schema).expect("valid schema");

	for raw in [
		"schema_version = 1",
		"schema_version = 2",
		"schema_version = '1'",
		"unknown = true",
		"[app]\nunknown = true",
		"[app]\nrender = 'spa'",
		"[app]\nversion = '1'",
		"[serve]\nport = 65536",
		"[serve]\nport = -1",
		"[build]\nwasm_opt = true",
		"[native]\ntargets = ['watchos']",
		"[native]\npackage_id = 'bad'",
		"[codepush]\nenabled = true",
		"[codepush]\npublic_keys = ['bad']",
		"[format]\nattribute_braces = 'sometimes'",
		"[[plugins]]\nunknown = true",
		"[[fns.targets]]",
		"[[fns.targets]]\nlanguage = 'ruby'",
		"[[fns.targets]]\nlanguage = 'rust'\nscaffold = 'unknown'",
	] {
		assert!(!validator.is_valid(&document(raw)), "accepted {raw}");
	}
}

#[test]
fn all_published_aliases_match_the_generated_contract() {
	let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/src");

	for asset in [
		"schema.json",
		"schemas/leptoskit.schema.json",
		"schemas/leptoskit.v0.schema.json",
	] {
		let id = format!("https://ifiokjr.github.io/leptoskit/{asset}");
		let expected = serde_json::to_value(schema::configuration(&id)).expect("schema");
		let raw = std::fs::read_to_string(root.join(asset)).expect("committed schema");
		let actual: Value = serde_json::from_str(&raw).expect("JSON schema");
		assert!(actual == expected, "stale {asset}; run schema:update");
	}
}
