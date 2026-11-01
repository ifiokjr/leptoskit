#![cfg(all(feature = "devtools", not(target_arch = "wasm32")))]

//! What leptoskit's `DevTools` read from a cache, and the edits they make.

mod common;

use std::time::Duration;

use common::Boom;
use common::Counter;
use common::quiet_client;
use common::wait_for;
use leptoskit_query::devtools::EditError;
use leptoskit_query::*;

fn notes() -> QueryScope<u32, Vec<String>, Boom> {
	QueryScope::new(|id| async move { Ok(vec![format!("note {id}")]) })
		.with_name("notes")
		.with_stale_time(Duration::from_secs(60))
		.with_ssr()
}

/// A scope without a codec, counting its fetches.
fn doubled(calls: Counter) -> QueryScope<u32, u32, Boom> {
	QueryScope::new(move |id| {
		let calls = calls.clone();
		async move {
			calls.hit();
			Ok(id * 2)
		}
	})
	.with_name("doubled")
	.with_stale_time(Duration::from_secs(60))
}

#[tokio::test]
async fn queries_are_listed_with_their_type_and_data() {
	let client = quiet_client();
	client.set_query_data(&notes(), 1, vec!["Field notes".to_owned()]);
	client.set_query_data(&doubled(Counter::new()), 4, 8);

	let mut queries = client.inspect_queries();
	queries.sort_by_key(|query| query.info().scope_name);
	let [doubled, notes] = queries.as_slice() else {
		panic!("two queries: {queries:?}");
	};

	assert_eq!(notes.info().key, "1");
	assert_eq!(notes.data_type(), "alloc::vec::Vec<alloc::string::String>");
	assert!(notes.has_codec());
	assert_eq!(
		notes.encode().unwrap().unwrap(),
		serde_json::json!(["Field notes"])
	);

	assert_eq!(doubled.data_type(), "u32");
	assert!(!doubled.has_codec());
	assert!(doubled.encode().is_none(), "no codec, no data");
}

#[tokio::test]
async fn data_is_replaced_through_the_codec() {
	let client = quiet_client();
	client.set_query_data(&notes(), 1, vec!["Field notes".to_owned()]);
	let query = client.inspect_queries().remove(0);

	query.set_json(serde_json::json!(["Edited"])).unwrap();
	assert_eq!(
		client.get_query_data(&notes(), &1),
		Some(vec!["Edited".to_owned()])
	);

	let error = query.set_json(serde_json::json!(7)).unwrap_err();
	assert!(matches!(error, EditError::Decode(_)));
	assert!(error.to_string().contains("refused"), "{error}");
	assert_eq!(
		client.get_query_data(&notes(), &1),
		Some(vec!["Edited".to_owned()])
	);
}

#[tokio::test]
async fn data_without_a_codec_is_read_only() {
	let client = quiet_client();
	client.set_query_data(&doubled(Counter::new()), 4, 8);
	let query = client.inspect_queries().remove(0);

	let error = query.set_json(serde_json::json!(9)).unwrap_err();
	assert!(matches!(error, EditError::NoCodec));
	assert!(error.to_string().contains("with_ssr"), "{error}");
	assert_eq!(client.get_query_data(&doubled(Counter::new()), &4), Some(8));
}

#[tokio::test]
async fn invalidating_marks_only_that_query_stale() {
	let client = quiet_client();
	client.set_query_data(&notes(), 1, vec!["one".to_owned()]);
	client.set_query_data(&notes(), 2, vec!["two".to_owned()]);
	let first = client
		.inspect_queries()
		.into_iter()
		.find(|query| query.info().key == "1")
		.unwrap();

	first.invalidate();
	let stale: Vec<(String, bool)> = {
		let mut infos = client.query_infos();
		infos.sort_by(|left, right| left.key.cmp(&right.key));
		infos
			.into_iter()
			.map(|info| (info.key, info.meta.is_invalidated))
			.collect()
	};
	assert_eq!(stale, [("1".to_owned(), true), ("2".to_owned(), false)]);
}

#[tokio::test]
async fn refetching_runs_even_an_unobserved_query() {
	let client = quiet_client();
	let calls = Counter::new();
	client.set_query_data(&doubled(calls.clone()), 4, 1);
	let query = client.inspect_queries().remove(0);

	query.refetch();
	wait_for("the refetch", || {
		client.get_query_data(&doubled(calls.clone()), &4) == Some(8)
	})
	.await;
	assert_eq!(calls.get(), 1);
}

#[tokio::test]
async fn mutations_are_listed_with_their_types() {
	let client = quiet_client();
	let scope: MutationScope<String, usize, Boom> =
		MutationScope::new(|title: String| async move { Ok(title.len()) }).with_name("count");
	let count = use_mutation_in(client.clone(), scope);
	assert_eq!(count.mutate_async("notes".to_owned()).await, Ok(5));

	let mutations = client.inspect_mutations();
	let [mutation] = mutations.as_slice() else {
		panic!("one mutation: {mutations:?}");
	};
	assert_eq!(mutation.info().scope_name, "count");
	assert_eq!(mutation.info().status, MutationStatus::Success);
	assert_eq!(mutation.variables_type(), "alloc::string::String");
	assert_eq!(mutation.data_type(), "usize");
}
