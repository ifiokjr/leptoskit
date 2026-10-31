//! Compile hostile wire names with Go, then inspect their parsed struct tags.
#![cfg(feature = "generators")]

use std::process::Command;

use leptoskit_rpc_codegen::ApiFunction;
use leptoskit_rpc_codegen::ApiParam;
use leptoskit_rpc_codegen::ApiSpec;
use leptoskit_rpc_codegen::GenerateOptions;
use leptoskit_rpc_codegen::HttpInput;
use leptoskit_rpc_codegen::HttpOutput;
use leptoskit_rpc_codegen::TargetLanguage;
use serde_json::json;

#[test]
fn go_wire_names_cannot_escape_struct_tags() {
	let names = [
		"back`tick",
		"line\nbreak",
		"carriage\rreturn",
		"quote\"name",
		"back\\slash",
		"comma,omitempty",
		"controls\0\u{0008}\u{000c}\t\u{001f}\u{007f}",
		"slash/name",
		"café",
		"field`\n}\nfunc injected() {}\ntype Extra struct {\nIgnored string `json:\"tail",
	];
	let mut spec = ApiSpec::empty("Tags", "0.0.0");
	let properties: serde_json::Map<String, serde_json::Value> = names
		.iter()
		.map(|name| ((*name).to_owned(), json!({ "type": "string" })))
		.collect();
	let object = json!({ "type": "object", "properties": properties });
	spec.schemas.insert("Payload".to_owned(), object.clone());
	spec.schemas.insert(
		"Nested".to_owned(),
		json!({ "type": "array", "items": object }),
	);
	for required in [true, false] {
		spec.functions.push(ApiFunction {
			kind: leptoskit_rpc_codegen::ApiKind::Call,
			name: if required {
				"send_required"
			} else {
				"send_optional"
			}
			.to_owned(),
			module_path: "api".to_owned(),
			struct_name: "Send".to_owned(),
			docs: String::new(),
			path: "/send".to_owned(),
			method: "POST".to_owned(),
			input: HttpInput::Json,
			output: HttpOutput::Json,
			error_content_type: "text/plain".to_owned(),
			params: names
				.iter()
				.map(|name| {
					ApiParam {
						name: (*name).to_owned(),
						docs: String::new(),
						schema: json!({ "type": "string" }),
						required,
					}
				})
				.collect(),
			output_schema: json!({ "$ref": "#/components/schemas/Payload" }),
			internal: false,
			deprecated: false,
			tags: Vec::new(),
		});
	}
	// Exercise the same JSON parsing boundary as a consumed RPC specification.
	let spec = ApiSpec::from_json(&serde_json::to_string(&spec).expect("serialize spec"))
		.expect("read spec");
	let generated = spec.generate(&GenerateOptions::new(TargetLanguage::Go, "tags"));
	let scratch = tempfile::tempdir().expect("Go package directory");

	for file in generated.files {
		std::fs::write(scratch.path().join(file.path), file.contents).expect("generated file");
	}

	std::fs::write(
		scratch.path().join("names.json"),
		serde_json::to_string(&names).expect("wire names"),
	)
	.expect("expected names");
	std::fs::write(
		scratch.path().join("tags_test.go"),
		include_str!("fixtures/tags_test.go"),
	)
	.expect("Go regression");
	let output = Command::new("go")
		.args(["test", "./..."])
		.env("GOWORK", "off")
		.env("GOPROXY", "off")
		.env("GOTOOLCHAIN", "local")
		.env("GOTELEMETRY", "off")
		.current_dir(scratch.path())
		.output()
		.expect("run Go (available in devenv)");
	assert!(
		output.status.success(),
		"Go rejected generated tags:\n{}\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
}
