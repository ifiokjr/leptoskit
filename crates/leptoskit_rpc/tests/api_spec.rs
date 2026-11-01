//! `#[api]` registration and spec collection, through real `#[server]`
//! functions compiled in server mode.
#![cfg(feature = "ssr")]
#![allow(deprecated)]
// `#[server]` functions are async by contract, awaited or not.
#![allow(clippy::unused_async)]

use http::Method;
use leptos::prelude::*;
use leptos::server_fn::codec::GetUrl;
use leptos::server_fn::codec::Json;
use leptoskit_rpc::api;
use leptoskit_rpc::spec;
use leptoskit_rpc::spec::HttpInput;
use leptoskit_rpc::spec::HttpOutput;
use leptoskit_rpc::spec::SpecOptions;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// One todo item.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Todo {
	/// The identifier.
	pub id: u64,
	/// What to do.
	pub title: String,
	/// Whether it is done.
	pub done: bool,
	/// Free-form labels.
	#[serde(default)]
	pub tags: Vec<String>,
}

/// Which todos to list.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TodoFilter {
	All,
	Done,
	Pending,
}

/// Something that happened to a todo.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Event {
	Created { id: u64 },
	Deleted { id: u64, reason: Option<String> },
}

/// Lists todos.
///
/// Newest first.
///
/// # Arguments
///
/// * `filter` - Which todos to list.
/// * `limit` - How many to return.
#[api(tag = "todos")]
#[server]
pub async fn list_todos(
	filter: TodoFilter,
	limit: Option<u32>,
) -> Result<Vec<Todo>, ServerFnError> {
	let _ = (filter, limit);
	Ok(Vec::new())
}

/// Creates a todo.
///
/// # Arguments
///
/// * `title` - What needs doing.
#[api(tag = "todos")]
#[server(input = Json)]
pub async fn create_todo(
	title: String,
	#[server(default)] tags: Vec<String>,
) -> Result<Todo, ServerFnError> {
	Ok(Todo {
		id: 1,
		title,
		done: false,
		tags,
	})
}

/// Reads one todo.
#[api]
#[server(input = GetUrl)]
pub async fn get_todo(id: u64) -> Result<Option<Todo>, ServerFnError> {
	let _ = id;
	Ok(None)
}

/// Flushes caches. For operators only.
#[api(internal)]
#[server]
pub async fn flush_caches() -> Result<(), ServerFnError> {
	Ok(())
}

/// Events since a point in time.
#[api(name = "events_since", tag = "events", tag = "legacy")]
#[deprecated(since = "0.1.0", note = "use the events stream")]
#[server(prefix = "/v1", endpoint = "events")]
pub async fn old_events(since: u64) -> Result<Vec<Event>, ServerFnError> {
	let _ = since;
	Ok(Vec::new())
}

/// The legacy positional `#[server]` form keeps its own route.
#[api(crate = "leptoskit_rpc")]
#[server(LegacyPing, "/legacy", "GetJson", "ping")]
pub async fn legacy_ping() -> Result<u8, ServerFnError> {
	Ok(1)
}

/// A server function without `#[api]`: private to the app.
#[server]
pub async fn hidden() -> Result<(), ServerFnError> {
	Ok(())
}

fn collect() -> spec::ApiSpec {
	spec::collect(&SpecOptions::new("Todos", "1.2.3").with_server("https://todos.example"))
		.expect("collect the spec")
}

#[test]
fn collects_only_api_functions_sorted_by_name() {
	let spec = collect();
	let names: Vec<&str> = spec
		.functions
		.iter()
		.map(|function| function.name.as_str())
		.collect();
	assert_eq!(
		names,
		[
			"create_todo",
			"events_since",
			"flush_caches",
			"get_todo",
			"legacy_ping",
			"list_todos"
		]
	);
}

#[test]
fn routes_are_stable_and_encodings_are_classified() {
	let spec = collect();
	let find = |name: &str| {
		spec.functions
			.iter()
			.find(|function| function.name == name)
			.expect(name)
	};

	let list = find("list_todos");
	assert_eq!(list.path, "/api/list_todos", "the endpoint is the fn name");
	assert_eq!(list.method, "POST");
	assert_eq!(list.input, HttpInput::Form);
	assert_eq!(list.output, HttpOutput::Json);
	assert_eq!(list.error_content_type, "text/plain");
	assert_eq!(list.docs, "Lists todos.\n\nNewest first.");
	assert_eq!(list.tags, ["todos"]);
	assert_eq!(list.struct_name, "ListTodos");
	assert!(list.module_path.ends_with("api_spec"));

	let get = find("get_todo");
	assert_eq!(get.method, "GET");
	assert_eq!(get.input, HttpInput::Query);

	let create = find("create_todo");
	assert_eq!(create.input, HttpInput::Json);
	assert!(create.params[0].required, "title is required");
	assert!(
		!create.params[1].required,
		"#[server(default)] makes tags optional"
	);

	let events = find("events_since");
	assert_eq!(events.path, "/v1/events", "explicit endpoints are kept");
	assert!(events.deprecated);
	assert_eq!(events.tags, ["events", "legacy"]);
	assert_eq!(events.struct_name, "OldEvents");

	let legacy = find("legacy_ping");
	assert_eq!(legacy.path, "/legacy/ping", "positional endpoints are kept");
	assert_eq!(legacy.struct_name, "LegacyPing");
	assert_eq!(legacy.input, HttpInput::Query, "GetJson is a GET");
}

#[test]
fn argument_docs_flow_from_the_arguments_section() {
	let spec = collect();
	let list = spec
		.functions
		.iter()
		.find(|function| function.name == "list_todos")
		.expect("list_todos");
	assert_eq!(list.params[0].docs, "Which todos to list.");
	assert_eq!(list.params[1].docs, "How many to return.");
	assert_eq!(list.params[0].name, "filter");
	assert!(list.params[0].is_required(), "filter is required");
	assert!(
		!list.params[1].is_required(),
		"limit is nullable, so optional on the wire"
	);

	let create = spec
		.functions
		.iter()
		.find(|function| function.name == "create_todo")
		.expect("create_todo");
	assert_eq!(create.params[0].docs, "What needs doing.");
	assert_eq!(
		create.params[1].docs, "",
		"undocumented arguments stay empty"
	);

	let object = spec::ApiSpec::params_object_schema(list);
	assert_eq!(
		object["properties"]["filter"]["description"],
		serde_json::json!("Which todos to list.")
	);
	let document = list_docs();
	assert!(
		document.contains("Which todos to list."),
		"the docs reach the OpenAPI document"
	);
}

fn list_docs() -> String {
	collect().public().to_openapi().to_string()
}

#[test]
fn internal_functions_leave_the_public_spec() {
	let spec = collect();
	assert!(spec.functions.iter().any(|function| function.internal));
	let public = spec.public();
	assert!(public.functions.iter().all(|function| !function.internal));
	assert!(
		!public
			.functions
			.iter()
			.any(|function| function.name == "flush_caches")
	);
	assert!(
		public.schemas.contains_key("Todo") && public.schemas.contains_key("Event"),
		"schemas used by public functions survive"
	);
}

#[test]
fn spec_and_openapi_snapshots() {
	let spec = collect();
	insta::assert_json_snapshot!("spec", spec);
	insta::assert_json_snapshot!("openapi", spec.public().to_openapi());
}

/// A hand-built descriptor, for exercising the collector without a macro.
fn descriptor(
	name: &'static str,
	path: &'static str,
	method: Method,
	input: &'static str,
	output: &'static str,
) -> spec::ApiFnDescriptor {
	spec::ApiFnDescriptor {
		kind: spec::ApiKind::Call,
		name,
		module_path: "hand::built",
		struct_name: "HandBuilt",
		docs: "",
		path,
		method,
		input_content_type: input,
		output_content_type: output,
		error_content_type: "application/json",
		params: &[spec::ApiParamDescriptor {
			name: "id",
			docs: "The identifier.",
			schema: |generator| generator.subschema_for::<u32>(),
			required: true,
		}],
		output_schema: |generator| generator.subschema_for::<String>(),
		internal: false,
		deprecated: false,
		tags: &[],
		scopes: &[],
	}
}

#[test]
fn collisions_are_reported() {
	let options = SpecOptions::new("x", "0");
	let form = "application/x-www-form-urlencoded";
	let json = "application/json";

	let a = descriptor("same", "/api/a", Method::POST, form, json);
	let b = descriptor("same", "/api/b", Method::POST, form, json);
	let error = spec::collect_from(&[&a, &b], &options).expect_err("same name");

	assert!(matches!(error, spec::SpecError::DuplicateName { ref name, .. } if name == "same"));
	assert!(
		error.to_string().contains("#[api(name = \"...\")]"),
		"{error}"
	);

	let a = descriptor("first", "/api/shared", Method::POST, form, json);
	let b = descriptor("second", "/api/shared", Method::POST, form, json);
	let error = spec::collect_from(&[&a, &b], &options).expect_err("same route");
	assert!(matches!(
		error,
		spec::SpecError::DuplicateRoute { ref method, ref path, .. } if method == "POST" && path == "/api/shared"
	));
	assert!(
		error.to_string().contains("`first` and `second`"),
		"{error}"
	);

	// Different methods on one path are two routes.
	let a = descriptor("read", "/api/thing", Method::GET, form, json);
	let b = descriptor("write", "/api/thing", Method::POST, form, json);
	assert_eq!(
		spec::collect_from(&[&a, &b], &options)
			.expect("distinct routes")
			.functions
			.len(),
		2
	);
}

#[test]
fn encodings_classify_by_method_and_content_type() {
	let options = SpecOptions::new("x", "0");
	let form = "application/x-www-form-urlencoded";
	let query = descriptor("q", "/api/q", Method::GET, form, "application/json");
	let json = descriptor(
		"j",
		"/api/j",
		Method::POST,
		"application/json",
		"application/json",
	);
	let cbor = descriptor(
		"c",
		"/api/c",
		Method::POST,
		"application/cbor",
		"application/cbor",
	);
	let spec = spec::collect_from(&[&query, &json, &cbor], &options).expect("collect");
	let by_name = |name: &str| {
		spec.functions
			.iter()
			.find(|function| function.name == name)
			.expect(name)
	};
	assert_eq!(by_name("q").input, HttpInput::Query);

	assert_eq!(by_name("j").input, HttpInput::Json);
	assert_eq!(
		by_name("c").input,
		HttpInput::Other {
			content_type: "application/cbor".to_owned()
		}
	);
	assert_eq!(
		by_name("c").output,
		HttpOutput::Other {
			content_type: "application/cbor".to_owned()
		}
	);
	assert_eq!(by_name("c").error_content_type, "application/json");
	let document = spec.to_openapi();
	let operation = &document["paths"]["/api/c"]["post"];
	assert_eq!(
		operation["x-leptoskit-unsupported-input"],
		"application/cbor"
	);
	assert_eq!(
		operation["x-leptoskit-unsupported-output"],
		"application/cbor"
	);
	assert_eq!(
		operation["responses"]["default"]["content"]["application/json"]
			["x-leptoskit-error-encoding"],
		"custom"
	);
	assert!(
		operation["responses"]["default"]["content"]["application/json"]["schema"]["type"]
			.is_null(),
		"a custom error type is not claimed to be a string"
	);
}

#[test]
fn options_come_from_the_environment_the_cli_sets() {
	let lookup = |name: &str| {
		match name {
			spec::SPEC_TITLE_ENV => Some("Todos".to_owned()),
			spec::SPEC_VERSION_ENV => Some("2.0.0".to_owned()),
			spec::SPEC_DESCRIPTION_ENV => Some(String::new()),
			spec::SPEC_SERVERS_ENV => Some(" https://a , ,https://b".to_owned()),
			_ => None,
		}
	};
	let options = SpecOptions::from_lookup(lookup);
	assert_eq!(options.title, "Todos");
	assert_eq!(options.version, "2.0.0");
	assert_eq!(options.description, None, "empty counts as unset");
	assert_eq!(options.servers, ["https://a", "https://b"]);

	let defaults = SpecOptions::from_lookup(|_| None);
	assert_eq!(defaults.title, "RPC");
	assert_eq!(defaults.version, "0.0.0");
	assert_eq!(defaults.servers, [] as [String; 0]);

	let built = SpecOptions::new("t", "1")
		.with_description("d")
		.with_server("https://s");
	assert_eq!(built.description.as_deref(), Some("d"));
	assert_eq!(built.servers, ["https://s"]);
	assert_eq!(
		SpecOptions::from_env().title,
		"RPC",
		"nothing exported in this process"
	);
}

#[test]
fn write_to_and_maybe_export_write_the_spec_file() {
	let dir = std::env::temp_dir().join(format!(
		"leptoskit_rpc_spec_{}_{}",
		std::process::id(),
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map(|duration| duration.as_nanos())
			.unwrap_or_default()
	));
	let path = dir.join("nested").join("spec.json");
	let options = SpecOptions::new("Todos", "1.2.3");
	let written = spec::write_to(&options, &path).expect("write");
	assert_eq!(written.title, "Todos");
	let reread =
		spec::ApiSpec::from_json(&std::fs::read_to_string(&path).expect("read")).expect("parse");
	assert_eq!(reread, written);

	assert!(spec::maybe_export(&options).is_none(), "nothing requested");
	assert!(
		spec::maybe_export_from(|_| Some(String::new()), &options).is_none(),
		"an empty path is no request"
	);
	let exported = dir.join("exported.json");
	let requested = exported.to_string_lossy().into_owned();
	let outcome = spec::maybe_export_from(
		|name| (name == spec::SPEC_OUT_ENV).then(|| requested.clone()),
		&options,
	)
	.expect("requested");
	assert_eq!(outcome.expect("exported"), exported);
	assert!(exported.is_file());

	let unwritable = path.join("child");
	let error = spec::write_to(&options, &unwritable).expect_err("a file is not a directory");
	assert!(matches!(error, spec::SpecError::Write { .. }), "{error}");
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn registry_is_sorted_by_name() {
	let names: Vec<&str> = spec::descriptors()
		.iter()
		.map(|descriptor| descriptor.name)
		.collect();
	let mut sorted = names.clone();
	sorted.sort_unstable();
	assert_eq!(names, sorted);
	assert!(names.contains(&"list_todos"));
}

#[cfg(feature = "axum")]
#[tokio::test]
async fn openapi_router_serves_json_and_yaml() {
	use http::Request;
	use http::StatusCode;
	use http::header::CONTENT_TYPE;
	use tower::ServiceExt;

	let router = spec::openapi_router(&SpecOptions::new("Todos", "1.2.3")).expect("router");
	let json = router
		.clone()
		.oneshot(
			Request::builder()
				.uri("/openapi.json")
				.body(axum::body::Body::empty())
				.expect("request"),
		)
		.await
		.expect("response");
	assert_eq!(json.status(), StatusCode::OK);
	assert_eq!(json.headers()[CONTENT_TYPE], "application/json");
	let body = axum::body::to_bytes(json.into_body(), usize::MAX)
		.await
		.expect("body");
	let document: serde_json::Value = serde_json::from_slice(&body).expect("json");
	assert_eq!(document["openapi"], "3.1.0");
	assert!(document["paths"]["/api/list_todos"].is_object());

	assert!(
		document["paths"]["/api/flush_caches"].is_null(),
		"the router serves the public spec"
	);

	let asyncapi = router
		.clone()
		.oneshot(
			Request::builder()
				.uri("/asyncapi.json")
				.body(axum::body::Body::empty())
				.expect("request"),
		)
		.await
		.expect("response");
	assert_eq!(asyncapi.status(), StatusCode::OK);
	let body = axum::body::to_bytes(asyncapi.into_body(), usize::MAX)
		.await
		.expect("body");
	let document: serde_json::Value = serde_json::from_slice(&body).expect("json");
	assert_eq!(document["asyncapi"], "3.0.0");
	assert!(document["channels"].is_object());

	let yaml = router
		.oneshot(
			Request::builder()
				.uri("/openapi.yaml")
				.body(axum::body::Body::empty())
				.expect("request"),
		)
		.await
		.expect("response");
	assert_eq!(yaml.headers()[CONTENT_TYPE], "application/yaml");
	let body = axum::body::to_bytes(yaml.into_body(), usize::MAX)
		.await
		.expect("body");
	assert!(String::from_utf8_lossy(&body).contains("\"openapi\": \"3.1.0\""));
}

#[test]
fn protocol_helpers_report_the_wire_shape() {
	// These are the const helpers the macro bakes into every descriptor;
	// calling them directly keeps the shapes honest.
	assert_eq!(spec::method_of::<ListTodos>(), Method::POST);
	assert_eq!(
		spec::input_content_type_of::<GetTodo>(),
		"application/x-www-form-urlencoded"
	);
	assert_eq!(
		spec::output_content_type_of::<ListTodos>(),
		"application/json"
	);
	assert_eq!(spec::error_content_type_of::<ListTodos>(), "text/plain");
}
