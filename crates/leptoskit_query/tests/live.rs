#![cfg(all(not(target_arch = "wasm32"), feature = "db"))]

//! Applying `leptoskit_db` live changes: exactly the queries tagged with a
//! changed row, or with its table, go stale.

mod common;

use std::time::Duration;

use common::Boom;
use common::quiet_client;
use leptoskit_query::QueryClient;
use leptoskit_query::QueryScope;
use leptoskit_query::Tag;
use leptoskit_query::live::ChangedRows;
use leptoskit_query::live::Invalidation;
use leptoskit_query::live::row_tag;
use leptoskit_query::live::table_tag;

/// Loads every query the test reads, fresh for a minute.
async fn loaded(client: &QueryClient, scope: &QueryScope<&'static str, u32, Boom>) {
	for key in ["posts", "post-1", "post-2", "tag-1", "other"] {
		client.fetch_query(scope, key).await.unwrap();
	}
}

fn scope() -> QueryScope<&'static str, u32, Boom> {
	QueryScope::new(|_: &'static str| async { Ok(1) })
		.with_name("live")
		.with_stale_time(Duration::from_secs(60))
		.with_tags_for(|key| {
			match *key {
				"posts" => vec![table_tag("posts")],
				"post-1" => vec![row_tag("posts", ["1"])],
				"post-2" => vec![row_tag("posts", ["2"])],
				"tag-1" => vec![row_tag("tags", ["1"])],
				_ => vec![Tag::new(["other"])],
			}
		})
}

fn stale(client: &QueryClient) -> Vec<String> {
	let mut stale: Vec<String> = client
		.query_infos()
		.into_iter()
		.filter(|info| info.is_stale)
		.map(|info| info.key)
		.collect();
	stale.sort();
	stale
}

#[tokio::test]
async fn a_changed_row_reaches_its_table_and_its_own_queries() {
	let client = quiet_client();
	let scope = scope();
	loaded(&client, &scope).await;
	assert_eq!(stale(&client), [] as [String; 0]);

	let invalidation = Invalidation::Rows(vec![ChangedRows {
		table: "posts".into(),
		keys: vec![vec!["1".into()]],
	}]);
	client.apply_invalidation(&invalidation).await;
	assert_eq!(stale(&client), ["\"post-1\"", "\"posts\""]);
}

#[tokio::test]
async fn everything_reaches_every_database_query_only() {
	let client = quiet_client();
	let scope = scope();
	loaded(&client, &scope).await;

	client.apply_invalidation(&Invalidation::Everything).await;
	assert_eq!(
		stale(&client),
		["\"post-1\"", "\"post-2\"", "\"posts\"", "\"tag-1\""]
	);
}

#[test]
fn tags_name_tables_and_rows() {
	assert_eq!(table_tag("posts"), Tag::new(["db", "posts"]));
	assert_eq!(
		row_tag("post_tags", ["1", "2"]),
		Tag::new(["db", "post_tags", "1", "2"])
	);
	assert_eq!(
		serde_json::to_string(&Invalidation::Everything).unwrap(),
		"\"everything\""
	);
	assert_eq!(
		serde_json::to_string(&Invalidation::Rows(vec![ChangedRows {
			table: "posts".into(),
			keys: vec![vec!["1".into()]],
		}]))
		.unwrap(),
		r#"{"rows":[{"table":"posts","keys":[["1"]]}]}"#
	);
}
