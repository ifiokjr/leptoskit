#![cfg(not(target_arch = "wasm32"))]

//! Infinite queries: paging forward and backward, trimming, and refetching.

mod common;

use std::time::Duration;

use common::Boom;
use common::Counter;
use common::owned;
use common::quiet_client;
use common::settle;
use common::wait_for;
use leptoskit_query::*;

/// A page of a numbered feed: `page_size` items starting at the cursor, with
/// cursors in both directions. `total` items exist in all.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Page {
	items: Vec<u32>,
	next: Option<u32>,
	previous: Option<u32>,
}

fn feed(calls: Counter, total: u32) -> InfiniteQueryScope<&'static str, u32, Page, Boom> {
	InfiniteQueryScope::new(
		move |ctx| {
			let calls = calls.clone();
			async move {
				calls.hit();
				tokio::time::sleep(Duration::from_millis(3)).await;
				let start = ctx.page_param;
				let end = (start + 3).min(total);
				Ok(Page {
					items: (start..end).collect(),
					next: (end < total).then_some(end),
					previous: (start > 0).then(|| start.saturating_sub(3)),
				})
			}
		},
		0,
	)
	.with_name("feed")
	.with_next_page_param(|ctx| ctx.page.next)
	.with_previous_page_param(|ctx| ctx.page.previous)
	.with_stale_time(Duration::from_secs(60))
}

#[tokio::test]
async fn pages_append_until_the_end() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = feed(calls.clone(), 7);

	let (owner, posts) = owned(|| {
		let client = client.clone();
		leptos::prelude::provide_context(client);
		use_infinite_query(scope.clone(), || "feed")
	});
	wait_for("first page", || posts.data().is_some()).await;
	assert_eq!(posts.pages().len(), 1);
	assert_eq!(posts.items_with(|page| &page.items), vec![0, 1, 2]);
	assert!(posts.has_next_page());
	assert!(!posts.has_previous_page());
	assert_eq!(posts.page_params(), vec![0]);

	posts.fetch_next_page().await;
	assert_eq!(posts.items_with(|page| &page.items), vec![0, 1, 2, 3, 4, 5]);
	assert!(posts.has_next_page());

	posts.fetch_next_page().await;
	assert_eq!(
		posts.items_with(|page| &page.items),
		vec![0, 1, 2, 3, 4, 5, 6]
	);
	assert!(!posts.has_next_page(), "the last page has no next cursor");
	assert_eq!(calls.get(), 3);

	posts.fetch_next_page().await;
	assert_eq!(calls.get(), 3, "no next page means no fetch");

	owner.cleanup();
}

#[tokio::test]
async fn previous_pages_prepend() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = feed(calls.clone(), 9).with_initial_page_param_fn(|_| 6);

	let (owner, posts) = owned(|| {
		leptos::prelude::provide_context(client.clone());
		use_infinite_query(scope.clone(), || "feed")
	});
	wait_for("first page", || posts.data().is_some()).await;
	assert_eq!(posts.items_with(|page| &page.items), vec![6, 7, 8]);
	assert!(posts.has_previous_page());
	assert!(!posts.has_next_page());

	posts.fetch_previous_page().await;
	assert_eq!(posts.items_with(|page| &page.items), vec![3, 4, 5, 6, 7, 8]);
	assert_eq!(posts.page_params(), vec![3, 6]);

	owner.cleanup();
}

#[tokio::test]
async fn max_pages_trims_the_far_end() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = feed(calls.clone(), 12).with_max_pages(2);

	let (owner, posts) = owned(|| {
		leptos::prelude::provide_context(client.clone());
		use_infinite_query(scope.clone(), || "feed")
	});
	wait_for("first page", || posts.data().is_some()).await;
	posts.fetch_next_page().await;
	posts.fetch_next_page().await;
	assert_eq!(posts.pages().len(), 2);
	assert_eq!(
		posts.items_with(|page| &page.items),
		vec![3, 4, 5, 6, 7, 8],
		"the first page was dropped"
	);
	assert!(
		posts.has_previous_page(),
		"trimmed pages can be paged back in"
	);

	posts.fetch_previous_page().await;
	assert_eq!(
		posts.items_with(|page| &page.items),
		vec![0, 1, 2, 3, 4, 5],
		"the last page was dropped"
	);

	owner.cleanup();
}

#[tokio::test]
async fn refetch_reloads_every_page_in_order() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = feed(calls.clone(), 9);

	let (owner, posts) = owned(|| {
		leptos::prelude::provide_context(client.clone());
		use_infinite_query(scope.clone(), || "feed")
	});
	wait_for("first page", || posts.data().is_some()).await;
	posts.fetch_next_page().await;
	posts.fetch_next_page().await;
	assert_eq!(posts.pages().len(), 3);
	assert_eq!(calls.get(), 3);

	let outcome = posts.refetch().await;
	assert!(matches!(outcome, Some(Ok(_))));
	assert_eq!(calls.get(), 6, "each loaded page is fetched again");
	assert_eq!(posts.pages().len(), 3);
	assert_eq!(
		posts.items_with(|page| &page.items),
		(0..9).collect::<Vec<_>>()
	);
	assert_eq!(posts.meta().data_update_count, 4);

	owner.cleanup();
}

#[tokio::test]
async fn direction_flags_track_the_fetch_in_flight() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = feed(calls.clone(), 9);

	let (owner, posts) = owned(|| {
		leptos::prelude::provide_context(client.clone());
		use_infinite_query(scope.clone(), || "feed")
	});
	wait_for("first page", || posts.data().is_some()).await;

	let next = posts.fetch_next_page();
	assert!(posts.is_fetching_next_page());
	assert!(!posts.is_fetching_previous_page());
	assert!(posts.is_refetching());
	next.await;
	assert!(!posts.is_fetching_next_page());
	assert!(!posts.is_fetch_next_page_error());

	owner.cleanup();
}

#[tokio::test]
async fn a_failing_page_reports_its_direction() {
	let client = quiet_client();
	let scope: InfiniteQueryScope<(), u32, u32, Boom> = InfiniteQueryScope::new(
		|ctx| {
			async move {
				if ctx.page_param == 0 {
					Ok(0)
				} else {
					Err(Boom("boundary"))
				}
			}
		},
		0,
	)
	.with_next_page_param(|ctx| Some(ctx.page_param + 1))
	.with_retry(Retry::never());

	let (owner, posts) = owned(|| {
		leptos::prelude::provide_context(client.clone());
		use_infinite_query(scope.clone(), || ())
	});
	wait_for("first page", || posts.data().is_some()).await;

	posts.fetch_next_page().await;
	assert!(posts.is_error());
	assert!(posts.is_fetch_next_page_error());
	assert!(!posts.is_fetch_previous_page_error());
	assert_eq!(
		posts.pages(),
		vec![0],
		"the loaded pages survive the failure"
	);
	assert!(posts.is_refetch_error());

	owner.cleanup();
}

#[tokio::test]
async fn client_helpers_fetch_pages_imperatively() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = feed(calls.clone(), 9);

	let data = client
		.fetch_infinite_query_pages(&scope, "feed", 2)
		.await
		.expect("two pages");
	assert_eq!(data.len(), 2);
	assert_eq!(
		data.items_with(|page| &page.items)
			.copied()
			.collect::<Vec<_>>(),
		vec![0, 1, 2, 3, 4, 5]
	);
	assert_eq!(calls.get(), 2);

	let again = client
		.fetch_infinite_query(&scope, "feed")
		.await
		.expect("cached");
	assert_eq!(again, data, "fresh pages are served from the cache");
	assert_eq!(calls.get(), 2);

	let ensured = client
		.ensure_infinite_query_data(&scope, "feed")
		.await
		.expect("cached");
	assert_eq!(ensured.len(), 2);

	client.set_infinite_query_data(
		&scope,
		"feed",
		InfiniteData {
			pages: vec![Page {
				items: vec![42],
				next: None,
				previous: None,
			}],
			page_params: vec![0],
		},
	);
	assert_eq!(
		client
			.get_query_data(&scope, &"feed")
			.map(|data| data.len()),
		Some(1)
	);

	let updated = client.update_infinite_query_data(&scope, "feed", |data| {
		let mut data = data?;
		data.pages[0].items.push(43);
		Some(data)
	});
	assert_eq!(
		updated.map(|data| data.pages[0].items.clone()),
		Some(vec![42, 43])
	);

	client.invalidate_scope(&scope).await;
	settle().await;
	assert_eq!(calls.get(), 2, "unobserved infinite queries only go stale");
}

#[tokio::test]
async fn infinite_data_round_trips_through_hydration() {
	let client = quiet_client();
	let calls = Counter::new();
	let scope = feed(calls.clone(), 6).with_name("feed").with_ssr();

	client
		.fetch_infinite_query_pages(&scope, "feed", 2)
		.await
		.expect("two pages");
	let dehydrated = client.dehydrate();
	assert_eq!(dehydrated.queries.len(), 1);

	let other = quiet_client();
	other.hydrate(dehydrated);
	let restored = other
		.ensure_infinite_query_data(&scope, "feed")
		.await
		.expect("hydrated");
	assert_eq!(restored.len(), 2);
	assert_eq!(calls.get(), 2, "hydrated pages are not fetched again");
}

#[tokio::test]
async fn cloning_an_infinite_scope_does_not_share_its_settings() {
	let client = quiet_client();
	let calls = Counter::new();
	let unbounded = feed(calls.clone(), 30);
	let bounded = unbounded.clone().with_max_pages(2);
	assert!(format!("{bounded:?}").contains("max_pages: Some(2)"));
	assert!(format!("{unbounded:?}").contains("max_pages: None"));

	let data = client
		.fetch_infinite_query_pages(&unbounded, "feed", 3)
		.await
		.expect("three pages");
	assert_eq!(
		data.len(),
		3,
		"the clone's page limit did not leak into the original"
	);
}
