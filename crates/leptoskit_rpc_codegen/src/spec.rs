//! The specification types: one plain-data description of the published RPC.

use std::collections::BTreeSet;

use indexmap::IndexMap;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

/// Marks a serialized [`ApiSpec`] so consumers can reject other JSON.
pub const SPEC_MARKER: &str = "leptoskit_rpc_spec";

/// The specification version this crate writes and reads.
pub const SPEC_VERSION: u32 = 1;

/// The environment variable naming the file the server writes its spec to
/// when asked (see `leptoskit_rpc::spec::export_if_requested`).
pub const SPEC_OUT_ENV: &str = "LEPTOSKIT_RPC_SPEC_OUT";
/// Overrides the spec title; set by the CLI from `leptoskit.toml`.
pub const SPEC_TITLE_ENV: &str = "LEPTOSKIT_RPC_SPEC_TITLE";
/// Overrides the spec version; set by the CLI from `leptoskit.toml`.
pub const SPEC_VERSION_ENV: &str = "LEPTOSKIT_RPC_SPEC_VERSION";
/// Overrides the spec description; set by the CLI from `leptoskit.toml`.
pub const SPEC_DESCRIPTION_ENV: &str = "LEPTOSKIT_RPC_SPEC_DESCRIPTION";
/// Overrides the server URLs, comma separated; set by the CLI from
/// `leptoskit.toml`.
pub const SPEC_SERVERS_ENV: &str = "LEPTOSKIT_RPC_SPEC_SERVERS";

/// Everything the tooling knows about one published server function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiFunction {
	/// The function name: the operation id, and the basis for client method
	/// and command names.
	pub name: String,
	/// Where the function lives, such as `my_app::api::todos`.
	pub module_path: String,
	/// The name of the struct the `#[server]` macro generates for the
	/// function, such as `ListTodos`.
	pub struct_name: String,
	/// The doc comment, with `///` markers stripped.
	#[serde(default)]
	pub docs: String,
	/// The route, beginning with a slash, such as `/api/list_todos`.
	pub path: String,
	/// The HTTP method, uppercase.
	pub method: String,
	/// How the request is encoded.
	pub input: HttpInput,
	/// How a successful response is encoded.
	pub output: HttpOutput,
	/// The content type errors come back as. The leptos default is
	/// `text/plain` holding `Kind|message`; custom error encodings say so
	/// here.
	pub error_content_type: String,
	/// The function's arguments, in declaration order.
	#[serde(default)]
	pub params: Vec<ApiParam>,
	/// The JSON Schema of the success value.
	pub output_schema: Value,
	/// Whether the function is internal: callable, just not publicized.
	/// Generated clients leave it out unless internal functions are asked
	/// for.
	#[serde(default)]
	pub internal: bool,
	/// Whether the function is deprecated.
	#[serde(default)]
	pub deprecated: bool,
	/// Grouping tags, from `#[api(tag = "...")]`.
	#[serde(default)]
	pub tags: Vec<String>,
}

/// One argument of an [`ApiFunction`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiParam {
	/// The argument name, as it appears on the wire.
	pub name: String,
	/// The argument's doc, from the `# Arguments` section of the function's
	/// doc comment.
	#[serde(default)]
	pub docs: String,
	/// The JSON Schema of the argument's type.
	pub schema: Value,
	/// Whether the argument must be sent. `#[server(default)]` arguments may
	/// be omitted, as may ones whose type admits `null`.
	#[serde(default = "default_true")]
	pub required: bool,
}

impl ApiParam {
	/// Whether a client must always send this argument.
	#[must_use]
	pub fn is_required(&self) -> bool {
		self.required && !schema_is_nullable(&self.schema)
	}
}

fn default_true() -> bool {
	true
}

/// How a function's request is encoded.
///
/// The variants carry just enough for the generators to build requests; any
/// other content type survives in [`HttpInput::Other`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HttpInput {
	/// Parameters in the query string, as `serde_qs` writes them.
	Query,
	/// A form-urlencoded body, as `serde_qs` writes it.
	Form,
	/// A JSON request body.
	Json,
	/// An encoding the generators do not know how to build.
	Other {
		/// The request content type.
		content_type: String,
	},
}

impl HttpInput {
	/// The request content type for this encoding.
	#[must_use]
	pub fn content_type(&self) -> &str {
		match self {
			Self::Query | Self::Form => "application/x-www-form-urlencoded",
			Self::Json => "application/json",
			Self::Other { content_type } => content_type,
		}
	}

	/// Whether a generated client can build the function's request.
	#[must_use]
	pub fn is_supported(&self) -> bool {
		!matches!(self, Self::Other { .. })
	}
}

/// How a successful response is encoded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HttpOutput {
	/// A JSON body.
	Json,
	/// An encoding the generators cannot decode.
	Other {
		/// The response content type.
		content_type: String,
	},
}

impl HttpOutput {
	/// The response content type for this encoding.
	#[must_use]
	pub fn content_type(&self) -> &str {
		match self {
			Self::Json => "application/json",
			Self::Other { content_type } => content_type,
		}
	}

	/// Whether a generated client can read the function's result.
	#[must_use]
	pub fn is_supported(&self) -> bool {
		!matches!(self, Self::Other { .. })
	}
}

/// The whole published RPC: metadata, the functions, and the shared type
/// schemas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiSpec {
	/// A marker plus version, so consumers can reject foreign JSON.
	pub leptoskit_rpc_spec: u32,
	/// The RPC's human name.
	pub title: String,
	/// The RPC's version.
	pub version: String,
	/// A longer description, if the project configured one.
	#[serde(default)]
	pub description: Option<String>,
	/// Base URLs the RPC is served from.
	#[serde(default)]
	pub servers: Vec<String>,
	/// The functions, sorted by name.
	pub functions: Vec<ApiFunction>,
	/// The named JSON Schemas referenced from the functions, keyed by schema
	/// name.
	#[serde(default)]
	pub schemas: IndexMap<String, Value>,
}

impl ApiSpec {
	/// An empty spec with the given title and version, for tests and seeds.
	#[must_use]
	pub fn empty(title: &str, version: &str) -> Self {
		Self {
			leptoskit_rpc_spec: SPEC_VERSION,
			title: title.to_owned(),
			version: version.to_owned(),
			description: None,
			servers: Vec::new(),
			functions: Vec::new(),
			schemas: IndexMap::new(),
		}
	}

	/// Serializes to pretty JSON.
	///
	/// # Errors
	///
	/// Returns an error when serialization fails, which for this plain-data
	/// type cannot happen in practice.
	pub fn to_json(&self) -> Result<String, CodegenError> {
		serde_json::to_string_pretty(self).map_err(CodegenError::Serialize)
	}

	/// Reads a spec written by [`ApiSpec::to_json`].
	///
	/// # Errors
	///
	/// Returns an error when the JSON is malformed, is not a leptoskit
	/// specification, or was written by an incompatible leptoskit version.
	pub fn from_json(json: &str) -> Result<Self, CodegenError> {
		let value: Value = serde_json::from_str(json).map_err(CodegenError::Deserialize)?;
		let found = value
			.get(SPEC_MARKER)
			.and_then(Value::as_u64)
			.ok_or(CodegenError::NotASpec)?;
		if found != u64::from(SPEC_VERSION) {
			return Err(CodegenError::Version {
				found,
				expected: SPEC_VERSION,
			});
		}
		serde_json::from_value(value).map_err(CodegenError::Deserialize)
	}

	/// The spec with internal functions removed and unreferenced schemas
	/// garbage-collected: what gets publicized.
	#[must_use]
	pub fn public(&self) -> Self {
		self.filtered(|function| !function.internal)
	}

	/// The spec with functions whose name matches an `exclude` glob removed,
	/// alongside the internal ones.
	#[must_use]
	pub fn excluding(&self, exclude: &[String]) -> Self {
		self.filtered(|function| {
			!function.internal
				&& !exclude
					.iter()
					.any(|glob| glob_matches(glob, &function.name))
		})
	}

	/// Builds a copy keeping the functions accepted by `keep`, then drops
	/// schemas nothing references anymore.
	#[must_use]
	pub fn filtered(&self, keep: impl Fn(&ApiFunction) -> bool) -> Self {
		let mut spec = self.clone();
		spec.functions.retain(keep);
		let mut reachable = BTreeSet::new();
		let mut stack: Vec<String> = Vec::new();
		for function in &spec.functions {
			let mut roots = BTreeSet::new();
			collect_refs(&function.output_schema, &mut roots);
			for param in &function.params {
				collect_refs(&param.schema, &mut roots);
			}
			stack.extend(roots);
		}
		while let Some(name) = stack.pop() {
			if !reachable.insert(name.clone()) {
				continue;
			}
			if let Some(schema) = self.schemas.get(&name) {
				let mut nested = BTreeSet::new();
				collect_refs(schema, &mut nested);
				stack.extend(nested);
			}
		}
		spec.schemas.retain(|name, _| reachable.contains(name));
		spec
	}

	/// The schema a `$ref` to `name` resolves to, if present.
	#[must_use]
	pub fn schema(&self, name: &str) -> Option<&Value> {
		self.schemas.get(name)
	}

	/// An object schema whose properties are the given function's parameters:
	/// exactly the shape the request body or query string carries.
	#[must_use]
	pub fn params_object_schema(function: &ApiFunction) -> Value {
		let properties: Map<String, Value> = function
			.params
			.iter()
			.map(|param| {
				let mut schema = param.schema.clone();
				if !param.docs.is_empty()
					&& let Some(object) = schema.as_object_mut()
				{
					object.insert("description".to_owned(), Value::String(param.docs.clone()));
				}
				(param.name.clone(), schema)
			})
			.collect();
		let required: Vec<Value> = function
			.params
			.iter()
			.filter(|param| param.is_required())
			.map(|param| Value::String(param.name.clone()))
			.collect();
		let mut object = Map::new();
		object.insert("type".to_owned(), Value::String("object".to_owned()));
		object.insert("properties".to_owned(), Value::Object(properties));
		if !required.is_empty() {
			object.insert("required".to_owned(), Value::Array(required));
		}
		Value::Object(object)
	}
}

/// Whether a schema admits `null`.
#[must_use]
pub fn schema_is_nullable(schema: &Value) -> bool {
	let Some(object) = schema.as_object() else {
		return false;
	};
	let typed_null = match object.get("type") {
		Some(Value::String(kind)) => kind == "null",
		Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind == "null"),
		_ => false,
	};
	if typed_null {
		return true;
	}
	if let Some(Value::Array(variants)) = object.get("anyOf") {
		return variants.iter().any(is_null_schema);
	}
	if let Some(Value::Array(variants)) = object.get("oneOf") {
		return variants.iter().any(is_null_schema);
	}
	false
}

fn is_null_schema(schema: &Value) -> bool {
	matches!(schema, Value::Object(map)
		if map.get("type") == Some(&Value::String("null".to_owned())))
}

/// Collects every schema name a schema references.
fn collect_refs(schema: &Value, refs: &mut BTreeSet<String>) {
	let Some(object) = schema.as_object() else {
		return;
	};
	if let Some(Value::String(reference)) = object.get("$ref")
		&& let Some(name) = ref_name(reference)
	{
		refs.insert(name.to_owned());
	}
	for key in ["items", "additionalProperties", "contains", "propertyNames"] {
		if let Some(nested) = object.get(key) {
			collect_refs(nested, refs);
		}
	}
	for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
		if let Some(Value::Array(variants)) = object.get(key) {
			for variant in variants {
				collect_refs(variant, refs);
			}
		}
	}
	if let Some(Value::Object(properties)) = object.get("properties") {
		for property in properties.values() {
			collect_refs(property, refs);
		}
	}
}

/// The schema name in a `$ref` pointing at the components, if it is one.
pub(crate) fn ref_name(reference: &str) -> Option<&str> {
	for prefix in ["/components/schemas/", "/$defs/"] {
		if let Some(name) = reference.strip_prefix(&format!("#{prefix}")) {
			return Some(name);
		}
	}
	None
}

/// Shell-style glob matching with `*` and `?`, as `rpc.exclude` uses it.
#[must_use]
pub fn glob_matches(pattern: &str, text: &str) -> bool {
	fn inner(pattern: &[char], text: &[char]) -> bool {
		match (pattern.first(), text.first()) {
			(None, None) => true,
			(Some('*'), _) => {
				inner(&pattern[1..], text) || (!text.is_empty() && inner(pattern, &text[1..]))
			}
			(Some('?'), Some(_)) => inner(&pattern[1..], &text[1..]),
			(Some(&expected), Some(&found)) => {
				expected == found && inner(&pattern[1..], &text[1..])
			}
			_ => false,
		}
	}
	let pattern: Vec<char> = pattern.chars().collect();
	let text: Vec<char> = text.chars().collect();
	inner(&pattern, &text)
}

/// Failures while reading a specification.
#[derive(Debug, thiserror::Error)]
pub enum CodegenError {
	/// The spec could not be serialized.
	#[error("could not serialize the RPC specification")]
	Serialize(#[source] serde_json::Error),
	/// The spec could not be parsed.
	#[error("could not parse the RPC specification: {0}")]
	Deserialize(serde_json::Error),
	/// The JSON carries no `leptoskit_rpc_spec` marker.
	#[error(
		"this JSON is not a leptoskit RPC specification; export one with `leptoskit rpc spec` or \
		 `leptoskit_rpc::spec::export_if_requested`"
	)]
	NotASpec,
	/// The spec was written by an incompatible leptoskit version.
	#[error(
		"the RPC specification is version {found}, but this tool reads version {expected}; \
		 regenerate the spec with a matching leptoskit"
	)]
	Version {
		/// The version found in the file.
		found: u64,
		/// The version this tool understands.
		expected: u32,
	},
	/// An unknown generator target was requested.
	#[error(
		"unknown target `{0}`; expected one of typescript, python, go, rust, dart, cli, openapi"
	)]
	UnknownTarget(String),
	/// An unknown scaffold mode was requested.
	#[error("unknown scaffold mode `{0}`; expected create, full, or files")]
	UnknownScaffold(String),
}

#[cfg(test)]
mod tests {
	use super::*;

	fn function(name: &str, refs: &[&str], internal: bool) -> ApiFunction {
		let schema_of = |reference: &str| serde_json::json!({ "$ref": format!("#/components/schemas/{reference}") });
		let struct_name = name
			.split('_')
			.map(|word| {
				let mut characters = word.chars();
				match characters.next() {
					Some(first) => first.to_ascii_uppercase().to_string() + characters.as_str(),
					None => String::new(),
				}
			})
			.collect::<String>();
		ApiFunction {
			name: name.to_owned(),
			module_path: "app".to_owned(),
			struct_name,
			docs: String::new(),
			path: format!("/api/{name}"),
			method: "GET".to_owned(),
			input: HttpInput::Query,
			output: HttpOutput::Json,
			error_content_type: "text/plain".to_owned(),
			params: refs
				.iter()
				.enumerate()
				.map(|(index, reference)| {
					ApiParam {
						name: format!("arg{index}"),
						docs: String::new(),
						schema: schema_of(reference),
						required: true,
					}
				})
				.collect(),
			output_schema: schema_of("Todo"),
			internal,
			deprecated: false,
			tags: Vec::new(),
		}
	}

	#[test]
	fn public_filters_internals_and_collects_schema_garbage() {
		let mut spec = ApiSpec::empty("demo", "0.1.0");
		spec.functions = vec![
			function("list_todos", &["Filter"], false),
			function("purge_cache", &["Cache"], true),
		];
		spec.schemas.insert(
			"Filter".to_owned(),
			serde_json::json!({ "type": "object", "properties": { "todo": { "$ref": "#/components/schemas/Todo" } } }),
		);
		spec.schemas
			.insert("Cache".to_owned(), serde_json::json!({ "type": "string" }));
		spec.schemas
			.insert("Unused".to_owned(), serde_json::json!({ "type": "string" }));
		spec.schemas
			.insert("Todo".to_owned(), serde_json::json!({ "type": "string" }));

		let public = spec.public();
		assert!(public.functions.iter().all(|f| !f.internal));
		let mut names: Vec<&String> = public.schemas.keys().collect();
		names.sort_unstable();
		assert_eq!(names, ["Filter", "Todo"]);
	}

	#[test]
	fn excluding_drops_globbed_names() {
		let mut spec = ApiSpec::empty("demo", "0.1.0");
		spec.functions = vec![
			function("debug_all", &[], false),
			function("debug_cache", &[], false),
			function("list_todos", &[], false),
		];
		let filtered = spec.excluding(&["debug_*".to_owned()]);
		let names: Vec<&str> = filtered.functions.iter().map(|f| f.name.as_str()).collect();
		assert_eq!(names, ["list_todos"]);
	}

	#[test]
	fn globs_match_stars_and_question_marks() {
		assert!(glob_matches("*", "anything"));
		assert!(glob_matches("debug_*", "debug_cache"));
		assert!(!glob_matches("debug_*", "list_todos"));
		assert!(glob_matches("get_?ser", "get_user"));
		assert!(!glob_matches("get_?ser", "get_users"));
	}

	#[test]
	fn round_trips_through_json() {
		let mut spec = ApiSpec::empty("demo", "1.2.3");
		spec.functions = vec![function("list_todos", &["Filter"], false)];
		let json = spec.to_json().expect("serialize");
		let back = ApiSpec::from_json(&json).expect("parse");
		assert_eq!(spec, back);
	}

	#[test]
	fn rejects_foreign_or_newer_json() {
		let error = ApiSpec::from_json("{}").expect_err("missing marker");
		assert!(matches!(error, CodegenError::NotASpec));
		let error =
			ApiSpec::from_json(&serde_json::json!({ "leptoskit_rpc_spec": 99 }).to_string())
				.expect_err("newer version");
		assert!(matches!(error, CodegenError::Version { found: 99, .. }));
		let error = ApiSpec::from_json("not json").expect_err("malformed");
		assert!(matches!(error, CodegenError::Deserialize(_)));
	}

	#[test]
	fn encodings_report_content_types_and_support() {
		assert_eq!(
			HttpInput::Query.content_type(),
			"application/x-www-form-urlencoded"
		);
		assert_eq!(
			HttpInput::Form.content_type(),
			"application/x-www-form-urlencoded"
		);
		assert_eq!(HttpInput::Json.content_type(), "application/json");
		let other = HttpInput::Other {
			content_type: "application/cbor".to_owned(),
		};
		assert_eq!(other.content_type(), "application/cbor");
		assert!(!other.is_supported() && HttpInput::Form.is_supported());
		assert_eq!(HttpOutput::Json.content_type(), "application/json");
		let other = HttpOutput::Other {
			content_type: "application/msgpack".to_owned(),
		};
		assert_eq!(other.content_type(), "application/msgpack");
		assert!(!other.is_supported() && HttpOutput::Json.is_supported());
	}

	#[test]
	fn nullability_is_detected_in_every_spelling() {
		assert!(schema_is_nullable(&serde_json::json!({ "type": "null" })));
		assert!(schema_is_nullable(
			&serde_json::json!({ "type": ["string", "null"] })
		));
		assert!(schema_is_nullable(
			&serde_json::json!({ "anyOf": [{ "type": "string" }, { "type": "null" }] })
		));
		assert!(schema_is_nullable(
			&serde_json::json!({ "oneOf": [{ "type": "null" }, { "type": "string" }] })
		));
		assert!(!schema_is_nullable(
			&serde_json::json!({ "oneOf": [{ "type": "integer" }, { "type": "string" }] })
		));
		assert!(!schema_is_nullable(
			&serde_json::json!({ "type": "string" })
		));
		assert!(!schema_is_nullable(&serde_json::json!(true)));
	}

	#[test]
	fn optional_parameters_are_not_required() {
		let optional = ApiParam {
			name: "limit".to_owned(),
			docs: "How many.".to_owned(),
			schema: serde_json::json!({ "type": ["integer", "null"] }),
			required: true,
		};
		assert!(!optional.is_required());
		let defaulted = ApiParam {
			name: "tags".to_owned(),
			docs: String::new(),
			schema: serde_json::json!({ "type": "array", "items": { "type": "string" } }),
			required: false,
		};
		assert!(!defaulted.is_required());
		let mut list = function("list", &[], false);
		list.params = vec![optional, defaulted];
		let object = ApiSpec::params_object_schema(&list);
		assert!(object.get("required").is_none(), "nothing is required");
		assert_eq!(
			object["properties"]["limit"]["description"],
			serde_json::json!("How many.")
		);
	}

	#[test]
	fn garbage_collection_follows_every_reference_keyword() {
		let reference =
			|name: &str| serde_json::json!({ "$ref": format!("#/components/schemas/{name}") });
		let mut spec = ApiSpec::empty("demo", "0.1.0");
		let mut root = function("root", &[], false);
		root.output_schema = serde_json::json!({ "allOf": [reference("ViaAllOf")] });
		spec.functions = vec![root];
		spec.schemas.insert(
			"ViaAllOf".to_owned(),
			serde_json::json!({ "items": reference("ViaItems") }),
		);
		spec.schemas.insert(
			"ViaItems".to_owned(),
			serde_json::json!({ "additionalProperties": reference("ViaMap") }),
		);
		spec.schemas.insert(
			"ViaMap".to_owned(),
			serde_json::json!({ "prefixItems": [reference("ViaTuple")] }),
		);
		spec.schemas.insert(
			"ViaTuple".to_owned(),
			serde_json::json!({ "properties": { "p": reference("ViaProperty") } }),
		);
		spec.schemas.insert(
			"ViaProperty".to_owned(),
			serde_json::json!({ "$ref": "#/$defs/ViaDefs" }),
		);
		spec.schemas.insert(
			"ViaDefs".to_owned(),
			serde_json::json!({ "type": "string" }),
		);
		spec.schemas
			.insert("Orphan".to_owned(), serde_json::json!({ "type": "string" }));
		let public = spec.public();
		let mut kept: Vec<&String> = public.schemas.keys().collect();
		kept.sort_unstable();
		assert_eq!(
			kept,
			[
				"ViaAllOf",
				"ViaDefs",
				"ViaItems",
				"ViaMap",
				"ViaProperty",
				"ViaTuple"
			]
		);
	}

	#[test]
	fn errors_explain_themselves() {
		assert!(
			CodegenError::NotASpec
				.to_string()
				.contains("leptoskit rpc spec")
		);
		assert!(
			CodegenError::UnknownTarget("x".to_owned())
				.to_string()
				.contains("typescript, python, go, rust, dart, cli, openapi")
		);
		assert!(
			CodegenError::UnknownScaffold("x".to_owned())
				.to_string()
				.contains("create, full, or files")
		);
		assert!(
			CodegenError::Version {
				found: 2,
				expected: 1
			}
			.to_string()
			.contains("version 2")
		);
	}
}
