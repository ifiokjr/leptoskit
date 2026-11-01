//! `use_live_paginated`: a long list a page at a time, every loaded page
//! live, with boundaries that stay put while rows move.
//!
//! It is a `leptoskit_query` infinite query whose pages are each a live
//! subscription. Loading more pins the last page at the row it ended on,
//! so the page after it starts there whatever is inserted or deleted
//! later; see [`Page`](crate::Page) for what the server does.

use std::sync::Arc;

use any_spawner::Executor;
use futures::StreamExt;
use leptos::prelude::*;
use leptoskit_query::InfiniteData;
use leptoskit_query::InfiniteQueryFnContext;
use leptoskit_query::InfiniteQueryHandle;
use leptoskit_query::InfiniteQueryScope;
use leptoskit_query::PageFetch;
use leptoskit_query::QueryClient;
use leptoskit_query::StaleTime;
use leptoskit_query::use_infinite_query;
use leptoskit_query::use_query_client;
use serde_json::Value;

use super::Args;
use super::FnError;
use super::call;
use super::live::client_if_any;
use crate::Page;
use crate::PageArgs;
use crate::Paginated;
use crate::live::Update;
use crate::live::apply;

type Scope<Q> = InfiniteQueryScope<Args<Q>, PageArgs, Page<<Q as Paginated>::Item>, FnError>;

/// What a paginated screen sees: every loaded row, whether more follow, and
/// a way to load them.
pub struct PaginatedHandle<T: Send + Sync + 'static> {
	pages: InfiniteQueryHandle<Page<T>, PageArgs, FnError>,
	pin: StoredValue<Arc<dyn Fn() + Send + Sync>>,
}

impl<T: Send + Sync + 'static> Clone for PaginatedHandle<T> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<T: Send + Sync + 'static> Copy for PaginatedHandle<T> {}

impl<T: Clone + Send + Sync + 'static> PaginatedHandle<T> {
	/// Every loaded row, page after page.
	pub fn items(&self) -> Vec<T> {
		self.pages.items_with(|page: &Page<T>| &page.items)
	}

	/// The loaded pages, first to last.
	pub fn pages(&self) -> Vec<Page<T>> {
		self.pages.pages()
	}

	/// Whether rows follow the last loaded page.
	pub fn has_more(&self) -> bool {
		self.pages.has_next_page()
	}

	/// Whether the next page is loading.
	pub fn is_loading_more(&self) -> bool {
		self.pages.is_fetching_next_page()
	}

	/// Loads the next page, after pinning the last one at the row it ended
	/// on. The fetch runs whether or not the returned [`PageFetch`] is
	/// awaited.
	pub fn load_more(&self) -> PageFetch {
		if self.has_more() {
			self.pin.with_value(|pin| pin());
		}

		self.pages.fetch_next_page()
	}

	/// The infinite query underneath, for its loading and error states.
	pub fn query(&self) -> InfiniteQueryHandle<Page<T>, PageArgs, FnError> {
		self.pages
	}
}

/// The infinite scope of a paginated query, named `rpc:<name>:pages`.
fn scope<Q: Paginated>(page_size: u32) -> Scope<Q> {
	InfiniteQueryScope::new(
		|context: InfiniteQueryFnContext<Args<Q>, PageArgs>| {
			call(context.key.get().clone().with_page(context.page_param))
		},
		PageArgs::first(page_size),
	)
	.with_next_page_param(move |context| {
		context
			.page
			.has_more
			.then(|| PageArgs::after(context.page.end.clone(), page_size))
	})
	.with_name(&format!("rpc:{}:pages", Q::NAME))
	.with_ssr()
}

/// A paginated live query: its pages a `page_size` at a time, each one
/// kept current from the socket, with `.load_more()` and `.has_more()`.
///
/// ```ignore
/// let history = rpc::use_live_paginated(move |page| History { channel: channel.get(), page }, 50);
/// let rows = move || history.items();
/// let more = move || history.has_more().then(|| view! { <Button on_press=move || { history.load_more(); }>"Older"</Button> });
/// ```
///
/// `args` builds the query for a page. The first page is fetched like any
/// query, over HTTP or in process while the server renders, and every page
/// then follows its own subscription, so a row inserted into page one
/// lengthens page one and leaves page two as it was. Without a socket it is
/// an ordinary infinite query.
#[track_caller]
pub fn use_live_paginated<Q>(
	args: impl Fn(PageArgs) -> Q + Send + Sync + 'static,
	page_size: u32,
) -> PaginatedHandle<Q::Item>
where
	Q: Paginated,
{
	let page_size = page_size.max(1);
	let args = Arc::new(args);
	let live = client_if_any().is_some();
	let mut scope = scope::<Q>(page_size);

	if live {
		// The socket keeps every page current: refetching would follow
		// cursors afresh and undo the pins.
		scope = scope.with_stale_time(StaleTime::Static);
	}

	let first = Arc::clone(&args);
	let key = ArcMemo::new(move |_| Args::new(first(PageArgs::first(page_size))));
	let read = key.clone();
	let pages = use_infinite_query(scope.clone(), move || read.get());
	let cache: QueryClient = use_query_client();

	let pin_scope = scope.clone();
	let pin_cache = cache.clone();
	let pin_key = key.clone();
	let pin: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
		pin_cache.update_infinite_query_data(&pin_scope, pin_key.get_untracked(), |data| {
			let mut data = data?;
			pin_last(&mut data);
			Some(data)
		});
	});

	if live {
		subscribe_pages::<Q>(key, pages, scope, cache);
	}

	PaginatedHandle {
		pages,
		pin: StoredValue::new(pin),
	}
}

/// Pins the last page at the row it ended on.
fn pin_last<T>(data: &mut InfiniteData<Page<T>, PageArgs>) {
	let (Some(page), Some(param)) = (data.pages.last(), data.page_params.last_mut()) else {
		return;
	};

	if !param.is_pinned() && page.end.is_some() {
		*param = param.clone().pinned(page.end.clone());
	}
}

/// Keeps one subscription per loaded page, each to that page's own
/// arguments, and writes its updates into that page alone.
fn subscribe_pages<Q: Paginated>(
	key: ArcMemo<Args<Q>>,
	pages: InfiniteQueryHandle<Page<Q::Item>, PageArgs, FnError>,
	scope: Scope<Q>,
	cache: QueryClient,
) {
	// Each loaded page's arguments, as JSON, and its subscription.
	let held = StoredValue::new(Vec::<(String, u32)>::new());

	Effect::new_isomorphic(move |_| {
		let key = key.get();
		let params = pages.page_params();
		let Some(live) = client_if_any() else {
			return;
		};
		let wanted: Vec<(String, PageArgs, Value)> = params
			.into_iter()
			.filter_map(|param| {
				let value =
					serde_json::to_value(key.get().clone().with_page(param.clone())).ok()?;
				Some((value.to_string(), param, value))
			})
			.collect();

		held.update_value(|held| {
			for index in 0..held.len().max(wanted.len()) {
				let same = matches!(
					(held.get(index), wanted.get(index)),
					(Some((had, _)), Some((want, _, _))) if had == want
				);

				if same {
					continue;
				}

				if let Some((_, id)) = held.get(index) {
					live.unsubscribe(*id);
				}

				let Some((text, param, value)) = wanted.get(index) else {
					continue;
				};
				let (id, updates) = live.subscribe(Q::NAME, value.clone());
				let entry = (text.clone(), id);

				if index < held.len() {
					held[index] = entry;
				} else {
					held.push(entry);
				}

				Executor::spawn_local(follow_page(
					cache.clone(),
					scope.clone(),
					key.clone(),
					(index, param.clone()),
					updates,
				));
			}

			held.truncate(wanted.len());
		});
	});

	on_cleanup(move || {
		let Some(live) = client_if_any() else {
			return;
		};
		let _ = held.try_update_value(|held| {
			for (_, id) in held.drain(..) {
				live.unsubscribe(id);
			}
		});
	});
}

/// Writes each update of the page at `index`, read with `param`, into its
/// place in the cache.
async fn follow_page<Q: Paginated>(
	cache: QueryClient,
	scope: Scope<Q>,
	key: Args<Q>,
	(index, param): (usize, PageArgs),
	mut updates: futures::channel::mpsc::UnboundedReceiver<Update>,
) {
	while let Some(update) = updates.next().await {
		cache.update_infinite_query_data(&scope, key.clone(), |data| {
			let mut data = data?;
			apply_to_page(&mut data, index, &param, &update);
			Some(data)
		});
	}
}

/// Applies a page's update, unless that page now reads other rows.
fn apply_to_page<T>(
	data: &mut InfiniteData<Page<T>, PageArgs>,
	index: usize,
	param: &PageArgs,
	update: &Update,
) where
	T: serde::Serialize + serde::de::DeserializeOwned,
{
	if data.page_params.get(index) != Some(param) || matches!(update, Update::Err(_)) {
		return;
	}

	let Some(page) = data.pages.get_mut(index) else {
		return;
	};
	let Ok(mut value) = serde_json::to_value(&*page) else {
		return;
	};
	apply(&mut value, update);

	if let Ok(next) = serde_json::from_value(value) {
		*page = next;
	}
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	fn data() -> InfiniteData<Page<String>, PageArgs> {
		InfiniteData {
			pages: vec![Page {
				items: vec!["a".to_owned()],
				end: Some("\"a\"".to_owned()),
				has_more: true,
			}],
			page_params: vec![PageArgs::first(1)],
		}
	}

	#[test]
	fn loading_more_pins_the_last_page_once() {
		let mut data = data();
		pin_last(&mut data);
		assert_eq!(data.page_params[0].until.as_deref(), Some("\"a\""));
		let pinned = data.page_params[0].clone();
		data.pages[0].end = Some("\"b\"".to_owned());
		pin_last(&mut data);
		assert_eq!(data.page_params[0], pinned, "a pinned page stays pinned");
	}

	#[test]
	fn an_update_reaches_only_its_own_page() {
		let mut data = data();
		let update = Update::Ok(json!({ "items": ["b"], "end": "\"b\"", "has_more": false }));
		apply_to_page(&mut data, 0, &PageArgs::first(2), &update);
		assert_eq!(data.pages[0].items, ["a"], "another page's arguments");
		apply_to_page(&mut data, 0, &PageArgs::first(1), &update);
		assert_eq!(data.pages[0].items, ["b"]);
		assert!(!data.pages[0].has_more);
	}
}
