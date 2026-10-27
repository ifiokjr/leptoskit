//! The harbour's RPC as other programs see it: the spec `#[api]` publishes,
//! the TypeScript client generated from it and committed under `clients/`,
//! and the server functions answering, refusing, and paging.

use std::path::Path;

use harbor_example::api;
use harbor_example::harbor::Harbour;
use harbor_example::harbor::LOG_PAGE;
use leptoskit::rpc::FnError;
use leptoskit::rpc::spec;
use leptoskit::rpc::spec::ApiSpec;
use leptoskit::rpc::spec::HttpInput;
use leptoskit::rpc::spec::SpecOptions;
use leptoskit_rpc_codegen::FileKind;
use leptoskit_rpc_codegen::GenerateOptions;
use leptoskit_rpc_codegen::TargetLanguage;

/// The spec as `leptoskit rpc export` writes it for this app.
fn harbour_spec() -> ApiSpec {
	spec::collect(&SpecOptions::new("Harbour", "0.1.0").with_server("https://harbour.example"))
		.expect("the harbour's functions describe themselves")
}

#[test]
fn the_spec_lists_every_function_on_its_own_route() {
	spec::check().expect("no two functions share a name or route");
	let spec = harbour_spec();
	let mut functions: Vec<(&str, &str, &str, &HttpInput)> = spec
		.functions
		.iter()
		.map(|function| {
			(
				function.name.as_str(),
				function.path.as_str(),
				function.method.as_str(),
				&function.input,
			)
		})
		.collect();
	functions.sort_by_key(|function| function.0);
	assert_eq!(
		functions,
		[
			("book_seat", "/api/book_seat", "POST", &HttpInput::Form),
			("harbor_log", "/api/harbor_log", "POST", &HttpInput::Form),
			("sailings", "/api/sailings", "POST", &HttpInput::Form),
		]
	);

	let book = spec
		.functions
		.iter()
		.find(|function| function.name == "book_seat")
		.expect("book_seat is published");
	assert!(book.docs.starts_with("Books one seat on a sailing"));
	assert_eq!(book.params[0].name, "sailing");
	assert_eq!(book.params[0].docs, "The sailing's id.");

	// The schemas carry the tagged status union the board renders.
	let openapi = spec.to_openapi().to_string();
	assert!(openapi.contains("SailingStatus"));
	assert!(openapi.contains("\"delayed\""));
}

#[test]
fn the_committed_typescript_client_is_what_the_spec_generates() {
	let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("clients/typescript");
	let generated = harbour_spec().generate(&GenerateOptions::new(
		TargetLanguage::TypeScript,
		"harbour-client",
	));

	let stale: Vec<String> = generated
		.files
		.iter()
		.filter(|file| file.kind == FileKind::Code)
		.filter(|file| {
			std::fs::read_to_string(directory.join(&file.path))
				.ok()
				.as_deref()
				!= Some(file.contents.as_str())
		})
		.map(|file| file.path.display().to_string())
		.collect();
	assert!(
		stale.is_empty(),
		"clients/typescript is out of date in {stale:?}; run `leptoskit rpc generate` in \
		 examples/harbor"
	);
}

#[tokio::test]
async fn the_server_functions_answer_refuse_and_page() {
	let sailings = api::sailings(5).await.expect("a day's sailings");
	assert_eq!(sailings.len(), 5);
	assert!(sailings.iter().all(|sailing| sailing.id / 100 == 5));

	let refused = api::book_seat(9_999).await.expect_err("no such sailing");
	assert_eq!(FnError::from(refused).message(), "no such sailing");
	let departed = api::book_seat(100).await.expect_err("the 06:40 has left");
	assert_eq!(FnError::from(departed).message(), "departed");

	// The log pages back from the newest entry to the first.
	let mut cursor = 0;
	let mut entries = Vec::new();
	loop {
		let page = api::harbor_log(cursor).await.expect("a log page");
		assert!(page.entries.len() <= LOG_PAGE as usize);
		entries.extend(page.entries.into_iter().map(|entry| entry.id));
		match page.next {
			Some(next) => cursor = next,
			None => break,
		}
	}
	assert_eq!(entries, (1..=10).rev().collect::<Vec<_>>());
	assert_eq!(Harbour::new().log_page(0).entries[0].id, 10);
}
