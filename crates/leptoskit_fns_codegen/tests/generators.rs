//! Generator snapshots: one fixture spec through every target.
#![cfg(feature = "generators")]

use std::collections::BTreeMap;
use std::fmt::Write as _;

use leptoskit_fns_codegen::ApiFunction;
use leptoskit_fns_codegen::ApiParam;
use leptoskit_fns_codegen::ApiSpec;
use leptoskit_fns_codegen::FileKind;
use leptoskit_fns_codegen::GenerateOptions;
use leptoskit_fns_codegen::HttpInput;
use leptoskit_fns_codegen::HttpOutput;
use leptoskit_fns_codegen::TargetLanguage;
use serde_json::json;

/// The spec every target renders: one function per encoding, one internal
/// function, and the schema shapes the generators model.
fn spec() -> ApiSpec {
	let mut schemas: BTreeMap<String, serde_json::Value> = BTreeMap::new();
	schemas.insert(
		"Todo".to_owned(),
		json!({
			"description": "One todo item.",
			"type": "object",
			"properties": {
				"done": { "type": "boolean", "description": "Whether it is done." },
				"id": { "type": "integer", "format": "uint64", "minimum": 0 },
				"tags": {
					"default": [],
					"description": "Free-form labels.",
					"items": { "type": "string" },
					"type": "array"
				},
				"title": { "type": "string" }
			},
			"required": ["id", "title", "done"]
		}),
	);
	schemas.insert(
		"TodoFilter".to_owned(),
		json!({
			"description": "Which todos to list.",
			"enum": ["all", "done", "pending"],
			"type": "string"
		}),
	);
	schemas.insert(
		"Event".to_owned(),
		json!({
			"description": "Something that happened to a todo.",
			"oneOf": [
				{
					"properties": {
						"id": { "type": "integer", "format": "uint64" },
						"kind": { "const": "created", "type": "string" }
					},
					"required": ["kind", "id"],
					"type": "object"
				},
				{
					"properties": {
						"id": { "type": "integer", "format": "uint64" },
						"kind": { "const": "deleted", "type": "string" },
						"reason": { "type": ["string", "null"] }
					},
					"required": ["kind", "id"],
					"type": "object"
				}
			]
		}),
	);
	schemas.insert(
		"Counts".to_owned(),
		json!({
			"type": "object",
			"additionalProperties": { "type": "integer" }
		}),
	);

	let function = |name: &str,
	                docs: &str,
	                path: &str,
	                method: &str,
	                input: HttpInput,
	                params: Vec<ApiParam>,
	                output_schema: serde_json::Value,
	                internal: bool| {
		ApiFunction {
			name: name.to_owned(),
			module_path: format!("todos::api::{name}"),
			struct_name: name
				.split('_')
				.map(|word| {
					let mut characters = word.chars();
					match characters.next() {
						Some(first) => first.to_ascii_uppercase().to_string() + characters.as_str(),
						None => String::new(),
					}
				})
				.collect::<String>(),
			docs: docs.to_owned(),
			path: path.to_owned(),
			method: method.to_owned(),
			input,
			output: HttpOutput::Json,
			error_content_type: "text/plain".to_owned(),
			params,
			output_schema,
			internal,
			deprecated: false,
			tags: Vec::new(),
		}
	};

	ApiSpec {
		leptoskit_fns_spec: 1,
		title: "Todos".to_owned(),
		version: "1.2.3".to_owned(),
		description: Some("The todo backend.".to_owned()),
		servers: vec!["https://todos.example".to_owned()],
		functions: vec![
			function(
				"list_todos",
				"Lists todos.\n\nNewest first.",
				"/api/list_todos",
				"POST",
				HttpInput::Form,
				vec![
					ApiParam {
						name: "filter".to_owned(),
						docs: "Which todos to list.".to_owned(),
						schema: json!({ "$ref": "#/components/schemas/TodoFilter" }),
						required: true,
					},
					ApiParam {
						name: "limit".to_owned(),
						docs: "How many to return.".to_owned(),
						schema: json!({
							"format": "uint32",
							"minimum": 0,
							"type": ["integer", "null"]
						}),
						required: true,
					},
				],
				json!({ "items": { "$ref": "#/components/schemas/Todo" }, "type": "array" }),
				false,
			),
			function(
				"create_todo",
				"Creates a todo.",
				"/api/create_todo",
				"POST",
				HttpInput::Json,
				vec![
					ApiParam {
						name: "title".to_owned(),
						docs: "What needs doing.".to_owned(),
						schema: json!({ "type": "string" }),
						required: true,
					},
					ApiParam {
						name: "tags".to_owned(),
						docs: String::new(),
						schema: json!({ "items": { "type": "string" }, "type": "array" }),
						required: false,
					},
				],
				json!({ "$ref": "#/components/schemas/Todo" }),
				false,
			),
			function(
				"get_todo",
				"Reads one todo.",
				"/api/get_todo",
				"GET",
				HttpInput::Query,
				vec![ApiParam {
					name: "id".to_owned(),
					docs: String::new(),
					schema: json!({
						"format": "uint64",
						"minimum": 0,
						"type": "integer"
					}),
					required: true,
				}],
				json!({
					"anyOf": [
						{ "$ref": "#/components/schemas/Todo" },
						{ "type": "null" }
					]
				}),
				false,
			),
			// Docless: the CLI generator synthesizes a summary either way.
			function(
				"counts",
				"",
				"/api/counts",
				"POST",
				HttpInput::Form,
				vec![],
				json!({ "$ref": "#/components/schemas/Counts" }),
				false,
			),
			function(
				"flush_caches",
				"Flushes caches. For operators only.",
				"/api/flush_caches",
				"POST",
				HttpInput::Form,
				vec![],
				json!({ "type": "null" }),
				true,
			),
		],
		schemas: schemas.into_iter().collect(),
	}
}

fn generate(language: TargetLanguage, package: &str) -> String {
	let mut spec = spec();
	// One supported, documented-but-deprecated function exercises the
	// deprecated branches every generator carries.
	for function in &mut spec.functions {
		if function.name == "counts" {
			function.deprecated = true;
		}
	}
	let generated = spec.generate(&GenerateOptions::new(language, package));
	let mut out = String::new();
	for file in &generated.files {
		let _ = writeln!(
			out,
			"===== {} ({:?}) =====\n{}",
			file.path.display(),
			file.kind,
			file.contents
		);
	}
	out
}

#[test]
fn openapi_snapshots() {
	insta::assert_snapshot!(
		"openapi",
		generate(TargetLanguage::OpenApi, "todos-openapi")
	);
}

#[test]
fn typescript_snapshots() {
	insta::assert_snapshot!(
		"typescript",
		generate(TargetLanguage::TypeScript, "@acme/todos")
	);
}

#[test]
fn python_snapshots() {
	insta::assert_snapshot!("python", generate(TargetLanguage::Python, "todos-api"));
}

#[test]
fn go_snapshots() {
	insta::assert_snapshot!("go", generate(TargetLanguage::Go, "example.com/todos"));
}

#[test]
fn rust_snapshots() {
	insta::assert_snapshot!("rust", generate(TargetLanguage::Rust, "todos-client"));
}

#[test]
fn dart_snapshots() {
	insta::assert_snapshot!("dart", generate(TargetLanguage::Dart, "todos"));
}

#[test]
fn cli_snapshots() {
	insta::assert_snapshot!("cli", generate(TargetLanguage::Cli, "todos-cli"));
}

#[test]
fn internal_functions_leave_the_generated_clients() {
	let spec = spec();
	let generated = spec.generate(&GenerateOptions::new(TargetLanguage::TypeScript, "todos"));
	let index = &generated
		.files
		.iter()
		.find(|file| file.path == std::path::Path::new("index.ts"))
		.expect("index.ts")
		.contents;
	assert!(
		!index.contains("flush_caches"),
		"internal functions stay out"
	);
	assert!(index.contains("listTodos("));
	assert!(
		index.contains("- `filter` — Which todos to list."),
		"{index}"
	);
}

#[test]
fn file_kinds_separate_code_from_scaffolding() {
	let spec = spec();
	let generated = spec.generate(&GenerateOptions::new(TargetLanguage::TypeScript, "todos"));
	let kind_of = |name: &str| {
		generated
			.files
			.iter()
			.find(|file| file.path == std::path::Path::new(name))
			.map(|file| file.kind)
			.expect(name)
	};
	assert_eq!(kind_of("index.ts"), FileKind::Code);
	assert_eq!(kind_of("package.json"), FileKind::Scaffold);
	assert_eq!(kind_of("tsconfig.json"), FileKind::Scaffold);
	assert_eq!(kind_of("README.md"), FileKind::Scaffold);
}

#[test]
fn exclude_and_internal_filter_together() {
	let options = GenerateOptions {
		exclude: vec!["get_*".to_owned()],
		..GenerateOptions::new(TargetLanguage::Python, "todos")
	};
	let generated = spec().generate(&options);
	let client = &generated
		.files
		.iter()
		.find(|file| file.path == std::path::Path::new("todos/client.py"))
		.expect("client.py")
		.contents;
	assert!(client.contains("def list_todos("));
	assert!(!client.contains("def get_todo("), "excluded by glob");
	assert!(!client.contains("flush_caches"), "internal stays out");
}

#[test]
fn unsupported_encodings_are_skipped_with_the_reason() {
	let mut spec = spec();
	spec.functions.push(ApiFunction {
		name: "blob".to_owned(),
		module_path: "todos::api".to_owned(),
		struct_name: "Blob".to_owned(),
		// No docs and deprecated: the CLI generator has branches for both.
		docs: String::new(),
		path: "/api/blob".to_owned(),
		method: "POST".to_owned(),
		input: HttpInput::Other {
			content_type: "application/cbor".to_owned(),
		},
		output: HttpOutput::Other {
			content_type: "application/cbor".to_owned(),
		},
		error_content_type: "text/plain".to_owned(),
		params: vec![],
		output_schema: json!({ "type": "string" }),
		internal: false,
		deprecated: true,
		tags: Vec::new(),
	});
	let generated = spec.generate(&GenerateOptions::new(TargetLanguage::TypeScript, "todos"));
	let cli = spec.generate(&GenerateOptions::new(TargetLanguage::Cli, "todos-cli"));
	let cli_main = &cli
		.files
		.iter()
		.find(|file| file.path == std::path::Path::new("src/main.rs"))
		.expect("main.rs")
		.contents;
	assert!(
		!cli_main.contains("blob("),
		"unsupported functions have no command"
	);
	let readme = &generated
		.files
		.iter()
		.find(|file| file.path == std::path::Path::new("README.md"))
		.expect("README")
		.contents;
	assert!(readme.contains("`blob`"), "{readme}");
	assert!(readme.contains("application/cbor"), "{readme}");
	let index = &generated
		.files
		.iter()
		.find(|file| file.path == std::path::Path::new("index.ts"))
		.expect("index.ts")
		.contents;
	assert!(!index.contains("blob("), "the function has no method");
}

#[test]
fn empty_specs_generate_valid_packages() {
	let spec = ApiSpec::empty("Nothing", "0.0.0");
	for language in TargetLanguage::ALL {
		let generated = spec.generate(&GenerateOptions::new(*language, "nothing"));
		assert!(!generated.files.is_empty(), "{language}");
		for file in &generated.files {
			assert!(
				!file.contents.is_empty(),
				"{language}: {}",
				file.path.display()
			);
		}
	}
}

/// The shapes the main fixture does not reach: tuples, anonymous objects (as
/// arguments and nested in properties), maps of references, inline enums,
/// case-colliding enum values, `const` strings, floats, free-form values,
/// `$ref`s to missing schemas, zero-argument functions, and a function whose
/// output encoding no client can decode.
fn edge_spec() -> ApiSpec {
	let mut schemas: BTreeMap<String, serde_json::Value> = BTreeMap::new();
	schemas.insert(
		"Point".to_owned(),
		json!({
			"description": "A pair.",
			"type": "array",
			"prefixItems": [{ "type": "number", "format": "double" }, { "type": "number" }],
			"items": false
		}),
	);
	schemas.insert(
		"Todo".to_owned(),
		json!({
			"type": "object",
			"properties": {
				"id": { "type": "integer", "format": "uint64" },
				"meta": {
					"type": "object",
					"properties": { "created-at": { "type": "string", "format": "date-time" } },
					"required": ["created-at"]
				},
				"kind": { "type": "string", "const": "todo" },
				"scores": { "type": "object", "additionalProperties": { "$ref": "#/components/schemas/Todo" } },
				"labels": { "type": "array", "items": { "type": "array", "items": { "type": "string" } } },
				"anything": {}
			},
			"required": ["id", "meta", "kind", "scores", "labels", "anything"]
		}),
	);
	schemas.insert(
		"Case".to_owned(),
		json!({ "type": "string", "enum": ["a", "A", "values"] }),
	);
	schemas.insert("Empty".to_owned(), json!({ "type": "object" }));
	schemas.insert(
		"Counts".to_owned(),
		json!({ "type": "object", "additionalProperties": { "type": "integer" } }),
	);

	ApiSpec {
		leptoskit_fns_spec: 1,
		title: "Edges".to_owned(),
		version: "0.0.1".to_owned(),
		description: None,
		servers: Vec::new(),
		functions: vec![
			ApiFunction {
				name: "move_point".to_owned(),
				module_path: "todos".to_owned(),
				struct_name: "MovePoint".to_owned(),
				docs: "Moves a point.".to_owned(),
				path: "/api/move_point".to_owned(),
				method: "POST".to_owned(),
				input: HttpInput::Form,
				output: HttpOutput::Json,
				error_content_type: "text/plain".to_owned(),
				params: vec![
					ApiParam {
						name: "to".to_owned(),
						docs: "Where to.".to_owned(),
						schema: json!({ "$ref": "#/components/schemas/Point" }),
						required: true,
					},
					ApiParam {
						name: "mode".to_owned(),
						docs: String::new(),
						schema: json!({ "type": "string", "enum": ["fast", "slow"] }),
						required: true,
					},
					ApiParam {
						name: "options".to_owned(),
						docs: String::new(),
						schema: json!({
							"type": "object",
							"properties": { "retries": { "type": "integer", "format": "uint8" }, "type": { "type": "boolean" } },
							"required": ["retries"]
						}),
						required: true,
					},
					ApiParam {
						name: "missing".to_owned(),
						docs: String::new(),
						schema: json!({ "$ref": "#/components/schemas/Missing" }),
						required: true,
					},
				],
				output_schema: json!({ "type": "null" }),
				internal: false,
				deprecated: false,
				tags: Vec::new(),
			},
			ApiFunction {
				name: "summarize".to_owned(),
				module_path: "todos".to_owned(),
				struct_name: "Summarize".to_owned(),
				docs: String::new(),
				path: "/api/summarize".to_owned(),
				method: "GET".to_owned(),
				input: HttpInput::Query,
				output: HttpOutput::Json,
				error_content_type: "text/plain".to_owned(),
				params: vec![ApiParam {
					name: "by_name".to_owned(),
					docs: String::new(),
					schema: json!({ "type": "object", "additionalProperties": { "$ref": "#/components/schemas/Case" } }),
					required: false,
				}],
				output_schema: json!({ "$ref": "#/components/schemas/Counts" }),
				internal: false,
				deprecated: false,
				tags: Vec::new(),
			},
			ApiFunction {
				name: "closed".to_owned(),
				module_path: "todos".to_owned(),
				struct_name: "Closed".to_owned(),
				docs: "Nothing to send.".to_owned(),
				path: "/api/closed".to_owned(),
				method: "POST".to_owned(),
				input: HttpInput::Form,
				output: HttpOutput::Json,
				error_content_type: "text/plain".to_owned(),
				params: vec![],
				output_schema: json!({ "$ref": "#/components/schemas/Case" }),
				internal: false,
				deprecated: false,
				tags: Vec::new(),
			},
			ApiFunction {
				name: "blob".to_owned(),
				module_path: "todos".to_owned(),
				struct_name: "Blob".to_owned(),
				docs: String::new(),
				path: "/api/blob".to_owned(),
				method: "POST".to_owned(),
				input: HttpInput::Form,
				output: HttpOutput::Other {
					content_type: "application/cbor".to_owned(),
				},
				error_content_type: "text/plain".to_owned(),
				params: vec![],
				output_schema: json!({ "type": "string" }),
				internal: false,
				deprecated: false,
				tags: Vec::new(),
			},
		],
		schemas: schemas.into_iter().collect(),
	}
}

#[test]
fn edge_shapes_render_in_every_target() {
	for language in TargetLanguage::ALL {
		let generated = edge_spec().generate(&GenerateOptions::new(*language, "edges"));
		let all = generated
			.files
			.iter()
			.map(|file| file.contents.as_str())
			.collect::<Vec<_>>()
			.join("\n");
		assert!(all.contains("move_point"), "{language}");
		// The output-decoding failure is reported, not silent.
		// The CLI has no README section; every other target records why.
		if !matches!(language, TargetLanguage::OpenApi | TargetLanguage::Cli) {
			assert!(
				all.contains("application/cbor"),
				"{language} records why `blob` is missing: {all}"
			);
		}
		if *language == TargetLanguage::Cli {
			assert!(
				!all.contains("Command::Blob"),
				"the CLI has no command for it"
			);
		}
	}
}

#[test]
fn degenerate_and_case_colliding_names_stay_valid() {
	let generated =
		edge_spec().generate(&GenerateOptions::new(TargetLanguage::TypeScript, "edges"));
	let index = &generated
		.files
		.iter()
		.find(|file| file.path == std::path::Path::new("index.ts"))
		.expect("index.ts")
		.contents;
	// The inline enum renders as a literal union; case-colliding values
	// survive; the tuple renders as a fixed-length array.
	assert!(index.contains("\"fast\" | \"slow\""), "{index}");
	assert!(index.contains("[number, number]"), "{index}");
	// A `$ref` to a schema the spec does not carry is `unknown`, not a name
	// nothing defines.
	assert!(index.contains("missing: unknown"), "{index}");

	let go = edge_spec().generate(&GenerateOptions::new(TargetLanguage::Go, "edges"));
	let models = &go
		.files
		.iter()
		.find(|file| file.path == std::path::Path::new("models.go"))
		.expect("models.go")
		.contents;
	assert!(
		models.contains("CaseA") && models.contains("CaseA2"),
		"case-colliding enum values get distinct constants: {models}"
	);
}
