//! Renders an [`ApiSpec`] as an OpenAPI 3.1 document.
//!
//! The document is plain `serde_json`: every OpenAPI toolchain reads JSON, and
//! 3.1 uses JSON Schema 2020-12 natively, which is exactly what the
//! specification carries. Functions map to operations keyed by route; internal
//! functions, when included, carry `x-leptoskit-internal: true` so downstream
//! generators can keep them out of public clients.

use serde_json::Map;
use serde_json::Value;
use serde_json::json;

use crate::spec::ApiFunction;
use crate::spec::ApiKind;
use crate::spec::ApiSpec;
use crate::spec::HttpInput;

/// The name of the security scheme scoped functions require.
const OAUTH2_SCHEME: &str = "oauth2";

/// The OpenAPI version this module writes.
pub fn openapi_version() -> &'static str {
	"3.1.0"
}

impl ApiSpec {
	/// Renders the spec as an OpenAPI 3.1 document.
	#[must_use]
	pub fn to_openapi(&self) -> Value {
		let mut paths = Map::new();
		for function in &self.functions {
			let operation = operation(function);
			let existing = paths
				.entry(function.path.clone())
				.or_insert_with(|| Value::Object(Map::new()));
			merge_operation(existing, &function.method, operation);
		}
		let mut document = Map::new();
		document.insert("openapi".to_owned(), json!(openapi_version()));
		let mut info = Map::new();
		info.insert("title".to_owned(), json!(self.title));
		info.insert("version".to_owned(), json!(self.version));
		if let Some(description) = &self.description {
			info.insert("description".to_owned(), json!(description));
		}
		document.insert("info".to_owned(), Value::Object(info));
		if !self.servers.is_empty() {
			let servers: Vec<Value> = self
				.servers
				.iter()
				.map(|url| json!({ "url": url }))
				.collect();
			document.insert("servers".to_owned(), Value::Array(servers));
		}
		let mut tags: Vec<String> = Vec::new();
		for function in &self.functions {
			for tag in &function.tags {
				if !tags.contains(tag) {
					tags.push(tag.clone());
				}
			}
		}
		if !tags.is_empty() {
			let tags: Vec<Value> = tags
				.into_iter()
				.map(|name| json!({ "name": name }))
				.collect();
			document.insert("tags".to_owned(), Value::Array(tags));
		}
		document.insert("paths".to_owned(), Value::Object(paths));
		let mut components = Map::new();
		if !self.schemas.is_empty() {
			let mut schemas = Map::new();
			for (name, schema) in &self.schemas {
				schemas.insert(name.clone(), schema.clone());
			}
			components.insert("schemas".to_owned(), Value::Object(schemas));
		}
		if let Some(scheme) = self.oauth2_scheme() {
			components.insert(
				"securitySchemes".to_owned(),
				json!({ OAUTH2_SCHEME: scheme }),
			);
		}
		if !components.is_empty() {
			document.insert("components".to_owned(), Value::Object(components));
		}
		Value::Object(document)
	}

	/// The `oauth2` security scheme, listing every scope a function asks
	/// for, when any does: third-party apps get tokens for them through the
	/// app's sign-in-with flow (`leptoskit_idp`), at the app's own
	/// `/oauth/authorize` and `/oauth/token`.
	fn oauth2_scheme(&self) -> Option<Value> {
		let mut scopes = Map::new();
		for function in &self.functions {
			for scope in &function.scopes {
				scopes.insert(scope.clone(), json!(""));
			}
		}
		if scopes.is_empty() {
			return None;
		}
		Some(json!({
			"type": "oauth2",
			"flows": {
				"authorizationCode": {
					"authorizationUrl": "/oauth/authorize",
					"tokenUrl": "/oauth/token",
					"refreshUrl": "/oauth/token",
					"scopes": scopes,
				}
			}
		}))
	}

	/// Renders the spec as an OpenAPI 3.1 document in YAML.
	#[must_use]
	pub fn to_openapi_yaml(&self) -> String {
		crate::yaml::to_yaml(&self.to_openapi())
	}
}

/// One operation, keyed by HTTP method under its path.
fn merge_operation(path_item: &mut Value, method: &str, operation: Value) {
	if let Some(object) = path_item.as_object_mut() {
		object.insert(method.to_ascii_lowercase(), operation);
	}
}

/// Builds the OpenAPI operation for one function.
fn operation(function: &ApiFunction) -> Value {
	let mut operation = Map::new();
	operation.insert("operationId".to_owned(), json!(function.name));
	if !function.docs.is_empty() {
		let (summary, description) = split_docs(&function.docs);
		operation.insert("summary".to_owned(), json!(summary));
		if let Some(description) = description {
			operation.insert("description".to_owned(), json!(description));
		}
	}
	if !function.tags.is_empty() {
		operation.insert("tags".to_owned(), json!(function.tags));
	}
	if function.deprecated {
		operation.insert("deprecated".to_owned(), json!(true));
	}
	if !function.scopes.is_empty() {
		operation.insert(
			"security".to_owned(),
			json!([{ OAUTH2_SCHEME: function.scopes }]),
		);
	}
	if function.internal {
		operation.insert("x-leptoskit-internal".to_owned(), json!(true));
	}
	if function.kind != ApiKind::Call {
		operation.insert("x-leptoskit-kind".to_owned(), json!(function.kind.as_str()));
	}
	if !function.input.is_supported() {
		operation.insert(
			"x-leptoskit-unsupported-input".to_owned(),
			json!(function.input.content_type()),
		);
	}
	if !function.output.is_supported() {
		operation.insert(
			"x-leptoskit-unsupported-output".to_owned(),
			json!(function.output.content_type()),
		);
	}

	let params_schema = ApiSpec::params_object_schema(function);
	match function.input {
		HttpInput::Query => {
			let parameters = query_parameters(function);
			operation.insert("parameters".to_owned(), Value::Array(parameters));
		}
		HttpInput::Form | HttpInput::Json | HttpInput::Other { .. } => {
			if function.kind.is_idempotent_with_key() {
				operation.insert(
					"parameters".to_owned(),
					json!([{
						"name": "Idempotency-Key",
						"in": "header",
						"required": false,
						"description": "Any unique string, such as a UUID. The first call with a \
										key runs; a repeat with the same key and arguments returns \
										the first call's result without running again.",
						"schema": { "type": "string", "maxLength": 255 },
					}]),
				);
			}
			let mut request = Map::new();
			request.insert("required".to_owned(), json!(true));
			let mut content = Map::new();
			content.insert(
				function.input.content_type().to_owned(),
				json!({ "schema": params_schema }),
			);
			request.insert("content".to_owned(), Value::Object(content));
			operation.insert("requestBody".to_owned(), Value::Object(request));
		}
	}

	let mut ok_content = Map::new();
	ok_content.insert(
		function.output.content_type().to_owned(),
		json!({ "schema": function.output_schema }),
	);
	let ok = json!({
		"description": success_description(function),
		"content": Value::Object(ok_content),
	});
	let mut error_content = Map::new();
	let encoding = error_encoding(function);
	// A custom error type's shape is not part of the spec, so its schema
	// promises nothing rather than a string it may not be.
	let error_schema = if encoding == "kind-pipe-message" {
		json!({ "type": "string" })
	} else {
		json!({ "description": "The function's own error type; its shape is not published." })
	};
	error_content.insert(
		function.error_content_type.clone(),
		json!({
			"schema": error_schema,
			"x-leptoskit-error-encoding": encoding,
		}),
	);
	let mut responses = Map::new();
	responses.insert("200".to_owned(), ok);
	responses.insert(
		"default".to_owned(),
		json!({
			"description": "An error; see the leptoskit error encoding",
			"content": Value::Object(error_content),
		}),
	);
	operation.insert("responses".to_owned(), Value::Object(responses));
	Value::Object(operation)
}

/// The leptos error wire shape a client should expect, when it is the default
/// `Kind|message` text.
fn error_encoding(function: &ApiFunction) -> &'static str {
	if function.error_content_type == "text/plain" {
		"kind-pipe-message"
	} else {
		"custom"
	}
}

fn success_description(function: &ApiFunction) -> String {
	if matches!(function.output, crate::spec::HttpOutput::Json) {
		"The function's return value.".to_owned()
	} else {
		format!(
			"The function's return value, encoded as {}.",
			function.output.content_type()
		)
	}
}

/// Spreads a query function's parameters into OpenAPI query parameters.
fn query_parameters(function: &ApiFunction) -> Vec<Value> {
	function
		.params
		.iter()
		.map(|param| {
			let mut parameter = Map::new();
			parameter.insert("name".to_owned(), json!(param.name));
			parameter.insert("in".to_owned(), json!("query"));
			if !param.docs.is_empty() {
				parameter.insert("description".to_owned(), json!(param.docs.trim()));
			}
			parameter.insert("required".to_owned(), json!(param.is_required()));
			parameter.insert("schema".to_owned(), param.schema.clone());
			Value::Object(parameter)
		})
		.collect()
}

/// Splits a doc comment into its first line and the rest.
fn split_docs(docs: &str) -> (String, Option<String>) {
	let mut lines = docs.lines();
	let summary = lines.next().unwrap_or_default().trim().to_owned();
	let rest = lines
		.map(str::trim)
		.collect::<Vec<&str>>()
		.join("\n")
		.trim()
		.to_owned();
	let description = (!rest.is_empty()).then_some(rest);
	(summary, description)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::spec::ApiParam;
	use crate::spec::HttpOutput;

	fn function(
		name: &str,
		docs: &str,
		input: HttpInput,
		params: Vec<ApiParam>,
		internal: bool,
	) -> ApiFunction {
		ApiFunction {
			kind: ApiKind::Call,
			name: name.to_owned(),
			module_path: "app".to_owned(),
			struct_name: "X".to_owned(),
			docs: docs.to_owned(),
			path: format!("/api/{name}"),
			method: "GET".to_owned(),
			input,
			output: HttpOutput::Json,
			error_content_type: "text/plain".to_owned(),
			params,
			output_schema: json!({ "type": "array", "items": { "type": "string" } }),
			internal,
			deprecated: false,
			tags: Vec::new(),
			scopes: Vec::new(),
		}
	}

	#[test]
	fn query_functions_document_parameters() {
		let mut spec = ApiSpec::empty("demo", "0.1.0");
		spec.functions = vec![function(
			"list_todos",
			"Lists todos.\n\nLonger description.",
			HttpInput::Query,
			vec![ApiParam {
				name: "limit".to_owned(),
				docs: "How many to return.".to_owned(),
				schema: json!({ "type": "integer", "format": "uint32", "minimum": 0 }),
				required: true,
			}],
			false,
		)];
		let document = spec.to_openapi();
		let operation = &document["paths"]["/api/list_todos"]["get"];
		assert_eq!(operation["operationId"], json!("list_todos"));
		assert_eq!(operation["summary"], json!("Lists todos."));
		assert_eq!(operation["description"], json!("Longer description."));
		assert_eq!(operation["parameters"][0]["name"], json!("limit"));
		assert_eq!(
			operation["parameters"][0]["description"],
			json!("How many to return.")
		);
		assert_eq!(operation["parameters"][0]["in"], json!("query"));
		assert_eq!(operation["parameters"][0]["required"], json!(true));
		assert_eq!(
			operation["responses"]["200"]["content"]["application/json"]["schema"],
			spec.functions[0].output_schema
		);
		assert_eq!(
			operation["responses"]["default"]["content"]["text/plain"]
				["x-leptoskit-error-encoding"],
			json!("kind-pipe-message")
		);
	}

	#[test]
	fn form_functions_document_a_request_body() {
		let mut spec = ApiSpec::empty("demo", "0.1.0");
		spec.functions = vec![function(
			"create_todo",
			"",
			HttpInput::Form,
			vec![ApiParam {
				name: "title".to_owned(),
				docs: String::new(),
				schema: json!({ "type": "string" }),
				required: true,
			}],
			true,
		)];
		let document = spec.to_openapi();
		let operation = &document["paths"]["/api/create_todo"]["get"];
		let body =
			&operation["requestBody"]["content"]["application/x-www-form-urlencoded"]["schema"];
		assert_eq!(body["properties"]["title"], json!({ "type": "string" }));
		assert_eq!(body["required"], json!(["title"]));
		assert_eq!(operation["x-leptoskit-internal"], json!(true));
	}

	#[test]
	fn scoped_functions_require_the_oauth2_scheme() {
		let mut spec = ApiSpec::empty("demo", "0.1.0");
		let mut scoped = function("list_posts", "", HttpInput::Json, Vec::new(), false);
		scoped.scopes = vec!["posts:read".to_owned()];
		spec.functions = vec![
			scoped,
			function("ping", "", HttpInput::Json, Vec::new(), false),
		];
		let document = spec.to_openapi();

		assert_eq!(
			document["paths"]["/api/list_posts"]["get"]["security"],
			json!([{ "oauth2": ["posts:read"] }])
		);
		assert!(
			document["paths"]["/api/ping"]["get"]
				.get("security")
				.is_none()
		);
		let flow =
			&document["components"]["securitySchemes"]["oauth2"]["flows"]["authorizationCode"];
		assert_eq!(flow["tokenUrl"], "/oauth/token");
		assert_eq!(flow["scopes"], json!({ "posts:read": "" }));

		let unscoped = ApiSpec::empty("demo", "0.1.0").to_openapi();
		assert!(unscoped.get("components").is_none());
	}

	#[test]
	fn unsupported_encodings_are_documented_not_dropped() {
		let mut spec = ApiSpec::empty("demo", "0.1.0");
		spec.description = Some("The demo.".to_owned());
		spec.servers = vec!["https://a".to_owned(), "https://b".to_owned()];
		spec.functions = vec![ApiFunction {
			kind: ApiKind::Call,
			name: "blob".to_owned(),
			module_path: "app".to_owned(),
			struct_name: "Blob".to_owned(),
			docs: String::new(),
			path: "/api/blob".to_owned(),
			method: "POST".to_owned(),
			input: HttpInput::Other {
				content_type: "application/cbor".to_owned(),
			},
			output: HttpOutput::Other {
				content_type: "application/cbor".to_owned(),
			},
			error_content_type: "application/json".to_owned(),
			params: vec![ApiParam {
				name: "bytes".to_owned(),
				docs: "The payload.".to_owned(),
				schema: json!({ "type": "string" }),
				required: true,
			}],
			output_schema: json!({ "type": "string" }),
			internal: false,
			deprecated: false,
			tags: vec!["a".to_owned(), "a".to_owned()],
			scopes: Vec::new(),
		}];
		let document = spec.to_openapi();
		assert_eq!(document["info"]["description"], "The demo.");
		assert_eq!(document["servers"][1]["url"], "https://b");
		assert_eq!(
			document["tags"].as_array().map(Vec::len),
			Some(1),
			"tags dedupe"
		);
		let operation = &document["paths"]["/api/blob"]["post"];
		assert_eq!(
			operation["x-leptoskit-unsupported-input"],
			"application/cbor"
		);
		assert_eq!(
			operation["x-leptoskit-unsupported-output"],
			"application/cbor"
		);
		assert!(operation["requestBody"]["content"]["application/cbor"].is_object());
		assert!(
			operation["responses"]["200"]["description"]
				.as_str()
				.is_some_and(|text| text.contains("application/cbor"))
		);
		assert_eq!(
			operation["responses"]["default"]["content"]["application/json"]
				["x-leptoskit-error-encoding"],
			"custom"
		);
		assert!(spec.to_openapi_yaml().contains("\"openapi\": \"3.1.0\""));
		assert_eq!(openapi_version(), "3.1.0");
	}

	#[test]
	fn queries_and_mutations_say_their_kind_and_take_an_idempotency_key() {
		let mut spec = ApiSpec::empty("demo", "0.1.0");
		let mut query = function("messages", "", HttpInput::Json, Vec::new(), false);
		query.kind = ApiKind::Query;
		query.method = "POST".to_owned();
		let mut mutation = function("send", "", HttpInput::Json, Vec::new(), false);
		mutation.kind = ApiKind::Mutation;
		mutation.method = "POST".to_owned();
		let mut action = function("summarize", "", HttpInput::Json, Vec::new(), false);
		action.kind = ApiKind::Action;
		action.method = "POST".to_owned();
		spec.functions = vec![query, mutation, action];
		let document = spec.to_openapi();

		let query = &document["paths"]["/api/messages"]["post"];
		assert_eq!(query["x-leptoskit-kind"], json!("query"));
		assert!(query.get("parameters").is_none());
		assert!(query["requestBody"]["content"]["application/json"].is_object());

		let mutation = &document["paths"]["/api/send"]["post"];
		assert_eq!(mutation["x-leptoskit-kind"], json!("mutation"));
		assert_eq!(mutation["parameters"][0]["name"], json!("Idempotency-Key"));
		assert_eq!(mutation["parameters"][0]["in"], json!("header"));
		assert_eq!(mutation["parameters"][0]["required"], json!(false));

		let action = &document["paths"]["/api/summarize"]["post"];
		assert_eq!(action["x-leptoskit-kind"], json!("action"));
		assert!(action.get("parameters").is_none());
	}
}
