//! Generator snapshots: one fixture spec through every target.
#![cfg(feature = "generators")]

use std::collections::BTreeMap;
use std::fmt::Write as _;

use leptoskit_rpc_codegen::ApiFunction;
use leptoskit_rpc_codegen::ApiKind;
use leptoskit_rpc_codegen::ApiParam;
use leptoskit_rpc_codegen::ApiSpec;
use leptoskit_rpc_codegen::FileKind;
use leptoskit_rpc_codegen::GenerateOptions;
use leptoskit_rpc_codegen::Generated;
use leptoskit_rpc_codegen::HttpInput;
use leptoskit_rpc_codegen::HttpOutput;
use leptoskit_rpc_codegen::TargetLanguage;
use serde_json::json;

#[test]
fn cli_help_metadata_cannot_add_commands() {
	let command_count = |spec: &ApiSpec| {
		let generated = spec.generate(&GenerateOptions::new(TargetLanguage::Cli, "safe-client"));
		let main = generated
			.files
			.iter()
			.find(|file| file.path == std::path::Path::new("src/main.rs"))
			.expect("CLI source");
		let syntax = syn::parse_file(&main.contents).expect("valid CLI source");
		syntax
			.items
			.into_iter()
			.find_map(|item| {
				match item {
					syn::Item::Enum(item) if item.ident == "Command" => Some(item.variants.len()),
					_ => None,
				}
			})
			.expect("commands")
	};
	let mut hostile_docs = spec();
	hostile_docs.functions[0].params[0].docs = "List mode.\nInjected,".to_owned();
	let mut hostile_name = spec();
	hostile_name.functions[0].docs.clear();
	hostile_name.functions[0].name = "list_todos\nInjected,".to_owned();
	let expected = command_count(&spec());

	for hostile in [hostile_docs, hostile_name] {
		assert_eq!(
			command_count(&hostile),
			expected,
			"metadata must not add command variants"
		);
	}
}

/// Metadata survives the remote JSON boundary as data, including
/// executable-looking text.
#[test]
fn hostile_metadata_cannot_change_generated_manifests_or_rust_items() {
	let payloads = [
		"quotes: \"\"\" ' \\ */ /* ${code}",
		"RPC\"\ntarget.'cfg(all())'.dependencies.injected = { path = 'injected' }\n#",
		"RPC\r\npub fn injected() {}\n#",
		"RPC\rpub fn injected() {}",
		"RPC\u{2028}pub fn injected() {}\u{2029}",
		"RPC\0\u{1}\u{7f}\u{85}\tend",
		"RPC\n[build-system]\nbuild-backend = 'injected'\n#",
	];

	for language in TargetLanguage::ALL {
		let options = GenerateOptions::new(*language, "safe-client");
		let baseline = spec().generate(&options);

		for payload in payloads {
			for field in ["title", "version"] {
				let mut document = serde_json::to_value(spec()).expect("spec JSON");
				document[field] = json!(payload);
				let hostile = ApiSpec::from_json(&document.to_string()).expect("remote spec");
				let generated = hostile.generate(&options);
				assert_metadata_outputs(&generated, &baseline, *language, &hostile);
			}
		}
	}
}

fn assert_metadata_outputs(
	generated: &Generated,
	baseline: &Generated,
	language: TargetLanguage,
	hostile: &ApiSpec,
) {
	for file in &generated.files {
		let original = baseline
			.files
			.iter()
			.find(|item| item.path == file.path)
			.expect("same files");

		if file
			.path
			.extension()
			.is_some_and(|extension| extension == "toml")
		{
			let mut actual: toml::Value = toml::from_str(&file.contents).expect("valid TOML");
			let mut expected: toml::Value =
				toml::from_str(&original.contents).expect("baseline TOML");
			let package = if language == TargetLanguage::Python {
				"project"
			} else {
				"package"
			};
			let prefix = if language == TargetLanguage::Cli {
				"A command line"
			} else {
				"A typed client"
			};
			assert_eq!(
				actual[package]["description"].as_str(),
				Some(format!("{prefix} for the {} RPC.", hostile.title).as_str())
			);
			assert_eq!(
				actual[package]["version"].as_str(),
				Some(hostile.version.as_str())
			);
			for key in ["description", "version"] {
				actual[package].as_table_mut().expect("package").remove(key);
				expected[package]
					.as_table_mut()
					.expect("package")
					.remove(key);
			}
			assert_eq!(
				actual, expected,
				"{language}: metadata must not change dependency or build configuration"
			);
		}

		if file
			.path
			.extension()
			.is_some_and(|extension| extension == "rs")
		{
			let actual = syn::parse_file(&file.contents).expect("valid Rust");
			let expected = syn::parse_file(&original.contents).expect("baseline Rust");
			assert_eq!(
				actual.items.len(),
				expected.items.len(),
				"metadata must not add executable items"
			);
		}

		if file
			.path
			.extension()
			.is_some_and(|extension| extension == "py")
		{
			let literal = file.contents.lines().nth(1).expect("module docstring");
			let docs: String = serde_json::from_str(literal).expect("one encoded string literal");
			assert!(docs.contains(&format!("{} {} RPC.", hostile.title, hostile.version)));
		}

		if file.path == std::path::Path::new("pubspec.yaml") {
			let mut actual = file.contents.lines().skip(1);
			let mut expected = original.contents.lines().skip(1);
			assert_eq!(actual.next(), expected.next());
			for (key, value) in [
				(
					"description",
					format!("A typed client for the {} RPC.", hostile.title),
				),
				("version", hostile.version.clone()),
			] {
				let line = actual.next().expect("metadata scalar");
				let scalar = line.strip_prefix(&format!("{key}: ")).expect("scalar key");
				assert_eq!(
					serde_json::from_str::<String>(scalar).expect("quoted YAML scalar"),
					value
				);
				expected.next();
			}
			assert_eq!(
				actual.collect::<Vec<_>>(),
				expected.collect::<Vec<_>>(),
				"metadata must not add YAML configuration"
			);
		}

		if file.kind == FileKind::Code && language != TargetLanguage::OpenApi {
			let banner = file.contents.lines().next().expect("banner");
			assert!(
				banner.ends_with("Do not edit; regenerate instead."),
				"{language}: {banner}"
			);
		}
	}
}

/// The spec every target renders: one function per encoding, one internal
/// function, and the schema shapes the generators model.
fn spec() -> ApiSpec {
	ApiSpec {
		leptoskit_rpc_spec: 2,
		title: "Todos".to_owned(),
		version: "1.2.3".to_owned(),
		description: Some("The todo backend.".to_owned()),
		servers: vec!["https://todos.example".to_owned()],
		functions: functions(),
		schemas: schemas().into_iter().collect(),
	}
}

/// The schema shapes the generators model: an object with defaults, a
/// string enum, a tagged union, and a map.
fn schemas() -> BTreeMap<String, serde_json::Value> {
	let mut schemas = BTreeMap::new();
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
	schemas
}

/// A public, undocumented function at `/api/<name>` with no parameters and
/// a null result, for each fixture to fill in.
fn api_function(name: &str, method: &str, input: HttpInput) -> ApiFunction {
	ApiFunction {
		kind: ApiKind::Call,
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
		docs: String::new(),
		path: format!("/api/{name}"),
		method: method.to_owned(),
		input,
		output: HttpOutput::Json,
		error_content_type: "text/plain".to_owned(),
		params: Vec::new(),
		output_schema: json!({ "type": "null" }),
		internal: false,
		deprecated: false,
		tags: Vec::new(),
		scopes: Vec::new(),
	}
}

fn param(name: &str, docs: &str, schema: serde_json::Value, required: bool) -> ApiParam {
	ApiParam {
		name: name.to_owned(),
		docs: docs.to_owned(),
		schema,
		required,
	}
}

/// One function per encoding, one docless, and one internal.
fn functions() -> Vec<ApiFunction> {
	vec![
		ApiFunction {
			docs: "Lists todos.\n\nNewest first.".to_owned(),
			params: vec![
				param(
					"filter",
					"Which todos to list.",
					json!({ "$ref": "#/components/schemas/TodoFilter" }),
					true,
				),
				param(
					"limit",
					"How many to return.",
					json!({
						"format": "uint32",
						"minimum": 0,
						"type": ["integer", "null"]
					}),
					true,
				),
			],
			output_schema: json!({ "items": { "$ref": "#/components/schemas/Todo" }, "type": "array" }),
			..api_function("list_todos", "POST", HttpInput::Form)
		},
		ApiFunction {
			docs: "Creates a todo.".to_owned(),
			params: vec![
				param(
					"title",
					"What needs doing.",
					json!({ "type": "string" }),
					true,
				),
				param(
					"tags",
					"",
					json!({ "items": { "type": "string" }, "type": "array" }),
					false,
				),
			],
			output_schema: json!({ "$ref": "#/components/schemas/Todo" }),
			..api_function("create_todo", "POST", HttpInput::Json)
		},
		ApiFunction {
			kind: ApiKind::Call,
			docs: "Reads one todo.".to_owned(),
			params: vec![param(
				"id",
				"",
				json!({
					"format": "uint64",
					"minimum": 0,
					"type": "integer"
				}),
				true,
			)],
			output_schema: json!({
				"anyOf": [
					{ "$ref": "#/components/schemas/Todo" },
					{ "type": "null" }
				]
			}),
			..api_function("get_todo", "GET", HttpInput::Query)
		},
		// Docless: the CLI generator synthesizes a summary either way.
		ApiFunction {
			output_schema: json!({ "$ref": "#/components/schemas/Counts" }),
			..api_function("counts", "POST", HttpInput::Form)
		},
		ApiFunction {
			docs: "Flushes caches. For operators only.".to_owned(),
			internal: true,
			..api_function("flush_caches", "POST", HttpInput::Form)
		},
		// The operation kinds: always JSON over `POST /api/<name>`.
		ApiFunction {
			kind: ApiKind::Query,
			docs: "Finds todos whose title contains the text.".to_owned(),
			params: vec![param(
				"text",
				"What to look for.",
				json!({ "type": "string" }),
				true,
			)],
			output_schema: json!({ "items": { "$ref": "#/components/schemas/Todo" }, "type": "array" }),
			..api_function("search_todos", "POST", HttpInput::Json)
		},
		ApiFunction {
			kind: ApiKind::Mutation,
			docs: "Marks a todo done.".to_owned(),
			params: vec![param(
				"id",
				"The todo.",
				json!({
					"format": "uint64",
					"minimum": 0,
					"type": "integer"
				}),
				true,
			)],
			output_schema: json!({ "$ref": "#/components/schemas/Todo" }),
			..api_function("complete_todo", "POST", HttpInput::Json)
		},
		ApiFunction {
			kind: ApiKind::Mutation,
			..api_function("clear_done", "POST", HttpInput::Json)
		},
	]
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
		kind: ApiKind::Call,
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
		scopes: Vec::new(),
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
		leptoskit_rpc_spec: 1,
		title: "Edges".to_owned(),
		version: "0.0.1".to_owned(),
		description: None,
		servers: Vec::new(),
		functions: vec![
			ApiFunction {
				kind: ApiKind::Call,
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
				scopes: Vec::new(),
			},
			ApiFunction {
				kind: ApiKind::Call,
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
				scopes: Vec::new(),
			},
			ApiFunction {
				kind: ApiKind::Call,
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
				scopes: Vec::new(),
			},
			ApiFunction {
				module_path: "todos".to_owned(),
				output: HttpOutput::Other {
					content_type: "application/cbor".to_owned(),
				},
				output_schema: json!({ "type": "string" }),
				..api_function("blob", "POST", HttpInput::Form)
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

/// Names that collide with what each client defines or leans on: components
/// named after runtime types (`Client`, `Error`, `Encoding`, `Command`) and
/// globals (`Record`, `Option`), one named like the Rust client's input
/// struct, functions named after transport methods (`call`, `new`,
/// `headers`, `to_string`), arguments named after locals and flags (`ctx`,
/// `self`, `result`, `server_url`), a field named after a Dart model method,
/// and an argument doc that would close a comment early.
fn colliding_spec() -> ApiSpec {
	let reference = |name: &str| json!({ "$ref": format!("#/components/schemas/{name}") });
	let mut schemas: BTreeMap<String, serde_json::Value> = BTreeMap::new();
	schemas.insert(
		"Client".to_owned(),
		json!({
			"type": "object",
			"properties": {
				"id": { "type": "integer", "format": "uint64" },
				"to_json": { "type": "string" }
			},
			"required": ["id", "to_json"]
		}),
	);
	schemas.insert(
		"Error".to_owned(),
		json!({ "type": "string", "enum": ["a", "b"] }),
	);
	schemas.insert(
		"Record".to_owned(),
		json!({ "type": "object", "additionalProperties": { "type": "string" } }),
	);
	schemas.insert(
		"Encoding".to_owned(),
		json!({ "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] }),
	);
	schemas.insert("Command".to_owned(), json!({ "type": "string" }));
	schemas.insert(
		"Option".to_owned(),
		json!({ "type": "object", "properties": { "on": { "type": "boolean" } }, "required": ["on"] }),
	);
	schemas.insert(
		"ListTodosInput".to_owned(),
		json!({ "type": "object", "properties": { "x": { "type": "boolean" } }, "required": ["x"] }),
	);

	let function = |name: &str, params: Vec<ApiParam>, output_schema: serde_json::Value| {
		ApiFunction {
			kind: ApiKind::Call,
			name: name.to_owned(),
			module_path: "clash".to_owned(),
			struct_name: "Clash".to_owned(),
			docs: String::new(),
			path: format!("/api/{name}"),
			method: "POST".to_owned(),
			input: HttpInput::Form,
			output: HttpOutput::Json,
			error_content_type: "text/plain".to_owned(),
			params,
			output_schema,
			internal: false,
			deprecated: false,
			tags: Vec::new(),
			scopes: Vec::new(),
		}
	};
	let param = |name: &str, docs: &str, schema: serde_json::Value| {
		ApiParam {
			name: name.to_owned(),
			docs: docs.to_owned(),
			schema,
			required: true,
		}
	};
	ApiSpec {
		leptoskit_rpc_spec: 1,
		title: "Clash".to_owned(),
		version: "0.0.1".to_owned(),
		description: None,
		servers: Vec::new(),
		functions: vec![
			function(
				"call",
				vec![
					param(
						"ctx",
						"Ends the comment */ early, then /* opens one.",
						json!({ "type": "string" }),
					),
					param("self", "", json!({ "type": "boolean" })),
					param("result", "", json!({ "type": "integer" })),
					param("server_url", "", json!({ "type": "string" })),
				],
				reference("Client"),
			),
			function("headers", vec![], reference("Error")),
			function(
				"new",
				vec![param("value", "", reference("Record"))],
				reference("Encoding"),
			),
			function(
				"list_todos",
				vec![
					param("filter", "", reference("Command")),
					param("option", "", reference("Option")),
				],
				reference("ListTodosInput"),
			),
			function("to_string", vec![], json!({ "type": "null" })),
		],
		schemas: schemas.into_iter().collect(),
	}
}

fn generate_colliding(language: TargetLanguage, package: &str) -> String {
	let generated = colliding_spec().generate(&GenerateOptions::new(language, package));
	let mut out = String::new();
	for file in generated
		.files
		.iter()
		.filter(|file| file.kind == FileKind::Code)
	{
		let _ = writeln!(
			out,
			"===== {} =====\n{}",
			file.path.display(),
			file.contents
		);
	}
	out
}

#[test]
fn typescript_names_clear_the_runtime() {
	let index = generate_colliding(TargetLanguage::TypeScript, "clash");
	assert!(index.contains("export interface Client2 {"), "{index}");
	assert!(
		index.contains("export type Record2 = Record<string, string>;"),
		"{index}"
	);
	assert!(index.contains("\tcall2(input: {"), "{index}");
	assert!(index.contains("\theaders2(): Promise<Error2>"), "{index}");
	assert!(index.contains("\ttoString2(): Promise<null>"), "{index}");
	assert!(
		index.contains("Ends the comment *\\/ early"),
		"argument docs are escaped: {index}"
	);
	assert!(
		!index.contains("comment */ early"),
		"no argument doc closes its comment: {index}"
	);
	insta::assert_snapshot!("typescript_collisions", index);
}

#[test]
fn python_names_clear_the_runtime() {
	let code = generate_colliding(TargetLanguage::Python, "clash");
	assert!(code.contains("class Client2(TypedDict"), "{code}");
	assert!(code.contains("def headers2(self)"), "{code}");
	assert!(code.contains("self2: bool"), "{code}");
	insta::assert_snapshot!("python_collisions", code);
}

#[test]
fn go_names_clear_the_runtime() {
	let code = generate_colliding(TargetLanguage::Go, "example.com/clash");
	assert!(code.contains("type Client2 struct {"), "{code}");
	assert!(code.contains("ctx2 string"), "{code}");
	assert!(code.contains("result2 int64"), "{code}");
	insta::assert_snapshot!("go_collisions", code);
}

#[test]
fn rust_names_clear_the_runtime() {
	let code = generate_colliding(TargetLanguage::Rust, "clash");
	assert!(code.contains("pub struct Client2 {"), "{code}");
	assert!(code.contains("pub enum Error2 {"), "{code}");
	assert!(code.contains("pub struct Option2 {"), "{code}");
	assert!(code.contains("struct ListTodosInput2 {"), "{code}");
	assert!(
		code.contains(
			"pub async fn call2(&self, ctx: String, self_: bool, result: i64, server_url: String) \
			 -> Result<Client2, Error>"
		),
		"an argument named `result` does not take the result's type: {code}"
	);
	assert!(code.contains("pub async fn new2(&self"), "{code}");
	assert!(code.contains("self_: bool"), "{code}");
	insta::assert_snapshot!("rust_collisions", code);
}

#[test]
fn cli_names_clear_clap_and_the_runtime() {
	let code = generate_colliding(TargetLanguage::Cli, "clash-cli");
	assert!(code.contains("pub type Command2 = String;"), "{code}");
	assert!(code.contains("server_url2: Json<String>"), "{code}");
	assert!(code.contains("run(api.call2("), "{code}");
	insta::assert_snapshot!("cli_collisions", code);
}

#[test]
fn dart_names_clear_the_runtime() {
	let code = generate_colliding(TargetLanguage::Dart, "clash");
	assert!(code.contains("class Client2 {"), "{code}");
	assert!(code.contains("final String toJson2;"), "{code}");
	assert!(code.contains("Future<void> toString2()"), "{code}");
	assert!(code.contains("required int result2"), "{code}");
	insta::assert_snapshot!("dart_collisions", code);
}

/// The Go client's encoders against the real `encoding/json`: numbers keep
/// every digit in the query string and in JSON bodies, and absent optional
/// fields leave JSON bodies.
#[test]
#[ignore = "needs the Go toolchain; run with --ignored"]
fn go_clients_keep_numbers_exact() {
	let dir = std::env::temp_dir().join(format!(
		"leptoskit_rpc_codegen_go_{}_{}",
		std::process::id(),
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map(|elapsed| elapsed.as_nanos())
			.unwrap_or_default()
	));
	for (name, spec) in [("todos", spec()), ("clash", colliding_spec())] {
		let package = dir.join(name);
		spec.generate(&GenerateOptions::new(
			TargetLanguage::Go,
			&format!("example.com/{name}"),
		))
		.write_to(&package, leptoskit_rpc_codegen::Scaffold::Full)
		.expect("write the package");
		let vet = std::process::Command::new("go")
			.arg("vet")
			.arg("./...")
			.current_dir(&package)
			.output()
			.expect("run go vet");
		assert!(
			vet.status.success(),
			"{name} vets clean: {}",
			String::from_utf8_lossy(&vet.stderr)
		);
	}
	std::fs::write(dir.join("todos/numbers_test.go"), GO_NUMBERS_TEST).expect("the test");
	let test = std::process::Command::new("go")
		.args(["test", "./..."])
		.current_dir(dir.join("todos"))
		.output()
		.expect("run go test");
	assert!(
		test.status.success(),
		"{}{}",
		String::from_utf8_lossy(&test.stdout),
		String::from_utf8_lossy(&test.stderr)
	);
	let _ = std::fs::remove_dir_all(&dir);
}

const GO_NUMBERS_TEST: &str = r#"package todos

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"
)

type numbers struct {
	ID     uint64   `json:"id"`
	Big    uint64   `json:"big"`
	Ratio  float64  `json:"ratio"`
	Labels []string `json:"labels"`
	Nested struct {
		N int64 `json:"n"`
	} `json:"nested"`
	Missing *string `json:"missing"`
}

func sample() numbers {
	value := numbers{ID: 1234567, Big: 9007199254740993, Ratio: 1.5, Labels: []string{"a"}}
	value.Nested.N = -42
	return value
}

func TestQueryValuesKeepNumbers(t *testing.T) {
	values, err := toQueryValues(sample())
	if err != nil {
		t.Fatal(err)
	}
	want := map[string]string{
		"id":        "1234567",
		"big":       "9007199254740993",
		"ratio":     "1.5",
		"labels[0]": "a",
		"nested[n]": "-42",
	}
	for key, expected := range want {
		if got := values.Get(key); got != expected {
			t.Errorf("%s = %q, want %q", key, got, expected)
		}
	}
	if values.Has("missing") {
		t.Errorf("nil fields are omitted: %v", values)
	}
}

func TestJSONBodiesKeepNumbers(t *testing.T) {
	var body string
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		bytes, _ := io.ReadAll(r.Body)
		body = string(bytes)
		_, _ = w.Write([]byte("null"))
	}))
	defer server.Close()
	if err := NewClient(server.URL).call(context.Background(), "POST", "/api/x", "json", sample()); err != nil {
		t.Fatal(err)
	}
	want := `{"big":9007199254740993,"id":1234567,"labels":["a"],"nested":{"n":-42},"ratio":1.5}`
	if body != want {
		t.Errorf("body = %s, want %s", body, want)
	}
}
"#;

#[test]
fn clis_whose_functions_need_scopes_can_sign_in() {
	let files = |spec: &ApiSpec, package: &str| {
		spec.generate(&GenerateOptions::new(TargetLanguage::Cli, package))
			.files
			.into_iter()
			.map(|file| (file.path.display().to_string(), file.contents))
			.collect::<BTreeMap<_, _>>()
	};

	// Without scopes there's nothing to sign in to.
	let plain = files(&spec(), "todos-cli");
	assert!(!plain.contains_key("src/login.rs"));
	assert!(!plain["src/main.rs"].contains("Login"));

	let mut scoped = spec();
	scoped.functions[0].scopes = vec!["todos:read".to_owned()];
	scoped.functions[1].scopes = vec!["todos:read".to_owned(), "todos:write".to_owned()];
	let generated = files(&scoped, "todos-cli");
	let login = &generated["src/login.rs"];
	assert!(
		login.contains("const APP: &str = \"todos-cli\";"),
		"{login}"
	);
	assert!(
		login.contains("const SCOPES: &str = \"todos:read todos:write\";"),
		"{login}"
	);
	assert!(
		login.contains("const CLIENT_ID_ENV: &str = \"TODOS_CLI_CLIENT_ID\";"),
		"{login}"
	);
	assert!(
		login.contains("urn:ietf:params:oauth:grant-type:device_code"),
		"{login}"
	);
	let main = &generated["src/main.rs"];
	assert!(main.contains("mod login;"), "{main}");
	assert!(main.contains("#[command(name = \"login\")]"), "{main}");
	assert!(
		main.contains("login::authorization(api.server_url())"),
		"{main}"
	);
	assert!(
		main.contains("Command::Login { .. } | Command::Logout =>"),
		"{main}"
	);
	assert!(generated["Cargo.toml"].contains("features = [\"rt\", \"time\"]"));
	assert!(generated["README.md"].contains("## Signing in"));
	assert!(generated["README.md"].contains("TODOS_CLI_CLIENT_ID"));

	// A function already called `login` keeps its name; signing in moves.
	scoped.functions[2].name = "login".to_owned();
	let generated = files(&scoped, "todos-cli");
	let main = &generated["src/main.rs"];
	assert!(main.contains("#[command(name = \"auth-login\")]"), "{main}");
	assert!(main.contains("#[command(name = \"login\")]"), "{main}");
	assert!(main.contains("#[command(name = \"logout\")]"), "{main}");
}

/// The signing-in CLI compiles. It needs clap, reqwest and tokio from the
/// registry, so it's run on request: `cargo test -- --ignored`.
#[test]
#[ignore = "builds a generated crate against the registry; run with --ignored"]
fn a_signing_in_cli_compiles() {
	let mut scoped = spec();
	scoped.functions[0].scopes = vec!["todos:read".to_owned()];
	let generated = scoped.generate(&GenerateOptions::new(TargetLanguage::Cli, "todos-cli"));
	let directory = tempfile::tempdir().unwrap();
	for file in &generated.files {
		let path = directory.path().join(&file.path);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, &file.contents).unwrap();
	}
	let output = std::process::Command::new(env!("CARGO"))
		.args(["check", "--quiet"])
		.current_dir(directory.path())
		.env("CARGO_TARGET_DIR", directory.path().join("target"))
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
}
