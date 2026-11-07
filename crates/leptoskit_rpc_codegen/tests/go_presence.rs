//! Run generated clients against a local Go HTTP server and round-trip models.
#![cfg(feature = "generators")]

use std::process::Command;

use leptoskit_rpc_codegen::ApiFunction;
use leptoskit_rpc_codegen::ApiKind;
use leptoskit_rpc_codegen::ApiParam;
use leptoskit_rpc_codegen::ApiSpec;
use leptoskit_rpc_codegen::GenerateOptions;
use leptoskit_rpc_codegen::HttpInput;
use leptoskit_rpc_codegen::HttpOutput;
use leptoskit_rpc_codegen::TargetLanguage;
use serde_json::json;

#[test]
fn go_distinguishes_omitted_values_from_explicit_zero_values() {
	let mut spec = ApiSpec::empty("Presence", "1.0.0");
	spec.schemas
		.insert("Flag".into(), json!({"type": "boolean"}));
	spec.schemas.insert(
		"Payload".into(),
		json!({
			"type": "object",
			"properties": {
				"enabled": {"type": "boolean", "default": true},
				"count": {"type": "integer", "default": 10},
				"text": {"type": "string", "default": "fallback"},
				"items": {"type": "array", "items": {"type": "string"}},
				"labels": {"type": "object", "additionalProperties": {"type": "string"}},
				"flag": {"$ref": "#/components/schemas/Flag"}
			}
		}),
	);
	for (name, input) in [
		("send", HttpInput::Json),
		("form", HttpInput::Form),
		("query", HttpInput::Query),
	] {
		spec.functions.push(ApiFunction {
			kind: ApiKind::Mutation,
			name: name.into(),
			module_path: "api".into(),
			struct_name: "Send".into(),
			docs: String::new(),
			path: format!("/{name}"),
			method: "POST".into(),
			input,
			output: HttpOutput::Json,
			error_content_type: "text/plain".into(),
			params: [
				("enabled", json!({"type": "boolean", "default": true})),
				("count", json!({"type": "integer", "default": 10})),
				("text", json!({"type": "string", "default": "fallback"})),
			]
			.into_iter()
			.map(|(name, schema)| {
				ApiParam {
					name: name.into(),
					docs: String::new(),
					schema,
					required: false,
				}
			})
			.collect(),
			output_schema: json!({"$ref": "#/components/schemas/Payload"}),
			internal: false,
			deprecated: false,
			tags: Vec::new(),
			scopes: Vec::new(),
		});
	}
	let dir = tempfile::tempdir().unwrap();
	for file in spec
		.generate(&GenerateOptions::new(TargetLanguage::Go, "presence"))
		.files
	{
		std::fs::write(dir.path().join(file.path), file.contents).unwrap();
	}
	std::fs::write(
		dir.path().join("presence_test.go"),
		include_str!("fixtures/presence_test.go"),
	)
	.unwrap();
	let output = Command::new("go")
		.args(["test", "./..."])
		.env("GOWORK", "off")
		.env("GOTOOLCHAIN", "local")
		.env("GOTELEMETRY", "off")
		.current_dir(dir.path())
		.output()
		.expect("run Go from devenv");
	assert!(
		output.status.success(),
		"generated Go presence test failed:\n{}\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
}
