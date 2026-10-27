//! The `#[server]` shapes `api_spec` does not cover, compiled for real in
//! server mode: every URL and JSON method, custom wrappers, raw identifiers,
//! `#[api(name)]` routes, skipped arguments, and argument doc comments.
#![cfg(feature = "ssr")]
// `#[server]` functions are async by contract, awaited or not.
#![allow(clippy::unused_async)]

use leptos::prelude::*;
use leptos::server_fn::codec::DeleteUrl;
use leptos::server_fn::codec::Json;
use leptos::server_fn::codec::PatchJson;
use leptos::server_fn::codec::PatchUrl;
use leptos::server_fn::codec::PutUrl;
use leptoskit_rpc::api;
use leptoskit_rpc::spec;
use leptoskit_rpc::spec::ApiFunction;
use leptoskit_rpc::spec::HttpInput;
use leptoskit_rpc::spec::SpecOptions;
use serde::Deserialize;
use serde::Serialize;

/// Deletes a todo.
#[api]
#[server(input = DeleteUrl)]
pub async fn delete_todo(id: u64) -> Result<(), ServerFnError> {
	let _ = id;
	Ok(())
}

/// Renames a todo.
#[api]
#[server(input = PatchUrl)]
pub async fn rename_todo(id: u64, title: String) -> Result<(), ServerFnError> {
	let _ = (id, title);
	Ok(())
}

/// Replaces a todo.
#[api]
#[server(input = PutUrl)]
pub async fn replace_todo(id: u64, title: String) -> Result<(), ServerFnError> {
	let _ = (id, title);
	Ok(())
}

/// Patches a todo with JSON.
#[api]
#[server(input = PatchJson)]
pub async fn patch_todo(id: u64, done: bool) -> Result<(), ServerFnError> {
	let _ = (id, done);
	Ok(())
}

/// A transparent wrapper of the kind `#[server(custom = ...)]` takes: leptos
/// implements `ServerFn` for `Traced<TracedPing>`, not `TracedPing`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Traced<T>(pub T);

/// Answers through a custom wrapper.
#[api]
#[server(custom = Traced)]
pub async fn traced_ping(note: String) -> Result<u8, ServerFnError> {
	let _ = note;
	Ok(1)
}

/// A keyword as an argument name, and as the published name, which is how
/// the raw-identifier diagnostic says to publish `fn r#type`.
#[api(name = "type")]
#[server]
pub async fn find_type(r#match: String) -> Result<String, ServerFnError> {
	Ok(r#match)
}

/// Counts todos under a published name of its own.
#[api(name = "todo_count")]
#[server]
pub async fn count() -> Result<u32, ServerFnError> {
	Ok(0)
}

/// Saves a note.
///
/// # Arguments
///
/// * `title` - What the note is called; shown in lists and in search results.
#[api]
#[server(input = Json)]
pub async fn save_note(
	title: String,
	/// The body, in Markdown.
	body: String,
	#[server(skip)] trace: Option<String>,
) -> Result<(), ServerFnError> {
	let _ = (title, body, trace);
	Ok(())
}

fn find(name: &str) -> ApiFunction {
	spec::collect(&SpecOptions::new("Shapes", "0.0.0"))
		.expect("collect the spec")
		.functions
		.into_iter()
		.find(|function| function.name == name)
		.unwrap_or_else(|| panic!("`{name}` is published"))
}

#[test]
fn delete_reads_the_query_string_and_the_rest_read_the_body() {
	let delete = find("delete_todo");
	assert_eq!(delete.method, "DELETE");
	assert_eq!(
		delete.input,
		HttpInput::Query,
		"DeleteUrl reads its arguments from the query string"
	);

	let rename = find("rename_todo");
	assert_eq!(rename.method, "PATCH");
	assert_eq!(rename.input, HttpInput::Form);

	let replace = find("replace_todo");
	assert_eq!(replace.method, "PUT");
	assert_eq!(replace.input, HttpInput::Form);

	let patch = find("patch_todo");
	assert_eq!(patch.method, "PATCH");
	assert_eq!(patch.input, HttpInput::Json);
}

#[test]
fn custom_wrappers_register_the_wrapped_server_fn() {
	let traced = find("traced_ping");
	assert_eq!(traced.path, "/api/traced_ping");
	assert_eq!(traced.struct_name, "TracedPing");
	assert_eq!(traced.params[0].name, "note");
	assert_eq!(traced.output_schema["format"], "uint8");
	assert_eq!(spec::method_of::<Traced<TracedPing>>(), http::Method::POST);
}

#[test]
fn raw_identifiers_drop_their_prefix_on_the_wire() {
	let function = find("type");
	assert_eq!(function.path, "/api/type");
	assert_eq!(function.struct_name, "FindType");
	assert_eq!(function.params[0].name, "match");
}

#[test]
fn a_published_name_is_also_the_route() {
	let count = find("todo_count");
	assert_eq!(count.path, "/api/todo_count");
	assert_eq!(count.struct_name, "Count");
}

#[test]
fn skipped_arguments_stay_off_the_wire_and_docs_reach_the_rest() {
	let save = find("save_note");
	let names: Vec<&str> = save
		.params
		.iter()
		.map(|param| param.name.as_str())
		.collect();
	assert_eq!(names, ["title", "body"], "the server never reads `trace`");
	assert_eq!(
		save.params[0].docs,
		"What the note is called; shown in lists and in search results."
	);
	assert_eq!(save.params[1].docs, "The body, in Markdown.");
	assert_eq!(save.docs, "Saves a note.");
}

#[test]
fn the_registry_passes_the_startup_check() {
	spec::check().expect("no two functions share a name or a route");
}
