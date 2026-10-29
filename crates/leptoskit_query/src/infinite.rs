//! Infinite queries: data that grows a page at a time in either direction.

use std::fmt;
use std::future::Future;
use std::ops::Deref;
use std::sync::Arc;

use leptos::prelude::*;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::awaitable::PageFetch;
use crate::cancel::CancelToken;
use crate::client::QueryClient;
use crate::client::use_query_client;
use crate::handle::IntoMaybeKey;
use crate::handle::QueryHandle;
use crate::handle::QueryObserverOptions;
use crate::handle::use_query_in;
use crate::key::ScopeId;
use crate::key::Tag;
use crate::options::GcTime;
use crate::options::QueryOptions;
use crate::options::RefetchInterval;
use crate::options::StaleTime;
use crate::query::FetchOptions;
use crate::retry::Retry;
use crate::retry::RetryDelay;
use crate::scope::DataCodec;
use crate::scope::FetchContext;
use crate::scope::Fetcher;
use crate::scope::QueryData;
use crate::scope::QueryError;
use crate::scope::QueryKey;
use crate::scope::QueryScope;
use crate::scope::ScopeLike;
use crate::state::FetchDirection;
use crate::time::BoxFuture;

/// The pages of an infinite query and the param each was fetched with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InfiniteData<T, P> {
	/// The pages, first to last.
	pub pages: Vec<T>,
	/// The page param each page was fetched with, in the same order.
	pub page_params: Vec<P>,
}

impl<T, P> Default for InfiniteData<T, P> {
	fn default() -> Self {
		Self {
			pages: Vec::new(),
			page_params: Vec::new(),
		}
	}
}

impl<T, P> InfiniteData<T, P> {
	/// No pages yet.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// How many pages are loaded.
	#[must_use]
	pub fn len(&self) -> usize {
		self.pages.len()
	}

	/// Whether no page is loaded.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.pages.is_empty()
	}

	/// The items of every page in order, for pages that are collections.
	pub fn items<'a, I>(&'a self) -> impl Iterator<Item = &'a I>
	where
		&'a T: IntoIterator<Item = &'a I>,
		I: 'a,
	{
		self.pages.iter().flat_map(IntoIterator::into_iter)
	}

	/// The items of every page in order, picked out of each page by `items` —
	/// for pages shaped like `{ items, next_cursor }`.
	pub fn items_with<'a, I>(
		&'a self,
		items: impl FnMut(&'a T) -> &'a [I],
	) -> impl Iterator<Item = &'a I>
	where
		I: 'a,
	{
		self.pages.iter().flat_map(items)
	}
}

/// What [`InfiniteQueryScope::with_next_page_param`] and
/// [`with_previous_page_param`](InfiniteQueryScope::with_previous_page_param)
/// decide from.
///
/// For the next page `page` is the last page loaded; for the previous page
/// it is the first.
#[derive(Debug)]
pub struct PageParamContext<'a, T, P> {
	/// The edge page.
	pub page: &'a T,
	/// Every page, first to last.
	pub pages: &'a [T],
	/// The edge page's param.
	pub page_param: &'a P,
	/// Every page's param, in the same order as `pages`.
	pub page_params: &'a [P],
}

/// What an infinite query's fetcher receives.
///
/// Derefs to the key.
#[derive(Clone, Debug)]
pub struct InfiniteQueryFnContext<K, P> {
	/// The key being fetched.
	pub key: K,
	/// The param of the page to fetch.
	pub page_param: P,
	/// Which end is being extended; `None` during a first load or refetch.
	pub direction: Option<FetchDirection>,
	/// Cancelled when the fetch is cancelled.
	pub signal: CancelToken,
}

impl<K, P> Deref for InfiniteQueryFnContext<K, P> {
	type Target = K;

	fn deref(&self) -> &K {
		&self.key
	}
}

type PageParamFn<T, P> = Arc<dyn Fn(PageParamContext<'_, T, P>) -> Option<P> + Send + Sync>;
type InitialParamFn<K, P> = Arc<dyn Fn(&K) -> P + Send + Sync>;
type PageFetcher<K, P, T, E> =
	Arc<dyn Fn(InfiniteQueryFnContext<K, P>) -> BoxFuture<'static, Result<T, E>> + Send + Sync>;

struct PageParams<T, P> {
	next: Option<PageParamFn<T, P>>,
	previous: Option<PageParamFn<T, P>>,
	max_pages: Option<usize>,
}

impl<T, P> Clone for PageParams<T, P> {
	fn clone(&self) -> Self {
		Self {
			next: self.next.clone(),
			previous: self.previous.clone(),
			max_pages: self.max_pages,
		}
	}
}

impl<T, P> PageParams<T, P> {
	fn next_param(&self, data: &InfiniteData<T, P>) -> Option<P> {
		let (page, page_param) = data.pages.last().zip(data.page_params.last())?;
		self.next.as_ref()?(PageParamContext {
			page,
			pages: &data.pages,
			page_param,
			page_params: &data.page_params,
		})
	}

	fn previous_param(&self, data: &InfiniteData<T, P>) -> Option<P> {
		let (page, page_param) = data.pages.first().zip(data.page_params.first())?;
		self.previous.as_ref()?(PageParamContext {
			page,
			pages: &data.pages,
			page_param,
			page_params: &data.page_params,
		})
	}

	fn add_page(&self, data: &mut InfiniteData<T, P>, page: T, param: P, backward: bool) {
		if backward {
			data.pages.insert(0, page);
			data.page_params.insert(0, param);
		} else {
			data.pages.push(page);
			data.page_params.push(param);
		}
		if let Some(max_pages) = self.max_pages {
			if data.pages.len() > max_pages {
				// Trim the end furthest from the one just extended.
				if backward {
					data.pages.pop();
					data.page_params.pop();
				} else {
					data.pages.remove(0);
					data.page_params.remove(0);
				}
			}
		}
	}
}

/// A [`QueryScope`] over pages.
///
/// Its data is an [`InfiniteData`] of pages. Fetching the next page appends
/// to the cached pages; a refetch reloads every page in order, following
/// cursors afresh; and `max_pages` trims from the far end — all as TanStack's
/// `useInfiniteQuery` does.
///
/// ```
/// use leptoskit_query::InfiniteQueryScope;
///
/// # #[derive(Clone, Debug)] struct Post;
/// # #[derive(Clone, Debug)] struct Page { posts: Vec<Post>, next_cursor: Option<u64> }
/// # #[derive(Clone, Debug)] struct ApiError;
/// # async fn fetch_feed(user: u32, cursor: u64) -> Result<Page, ApiError> { let _ = (user, cursor); Ok(Page { posts: vec![], next_cursor: None }) }
/// fn feed() -> InfiniteQueryScope<u32, u64, Page, ApiError> {
/// 	InfiniteQueryScope::new(
/// 		|ctx| async move { fetch_feed(ctx.key, ctx.page_param).await },
/// 		0,
/// 	)
/// 	.with_next_page_param(|ctx| ctx.page.next_cursor)
/// 	.with_max_pages(5)
/// }
/// ```
///
/// Builders change only the scope they return: a clone taken earlier keeps
/// its own settings.
pub struct InfiniteQueryScope<K, P, T, E> {
	scope: QueryScope<K, InfiniteData<T, P>, E>,
	/// The caller's fetcher for one page; `scope`'s fetcher pages with it
	/// under `params` and `initial`, and is rebuilt whenever they change.
	fetch_page: PageFetcher<K, P, T, E>,
	params: Arc<PageParams<T, P>>,
	initial: InitialParamFn<K, P>,
}

impl<K, P, T, E> Clone for InfiniteQueryScope<K, P, T, E> {
	fn clone(&self) -> Self {
		Self {
			scope: self.scope.clone(),
			fetch_page: Arc::clone(&self.fetch_page),
			params: Arc::clone(&self.params),
			initial: Arc::clone(&self.initial),
		}
	}
}

impl<K, P, T, E> fmt::Debug for InfiniteQueryScope<K, P, T, E> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("InfiniteQueryScope")
			.field("scope", &self.scope)
			.field("max_pages", &self.params.max_pages)
			.finish_non_exhaustive()
	}
}

impl<K, P, T, E> ScopeLike for InfiniteQueryScope<K, P, T, E>
where
	K: QueryKey,
	P: QueryData,
	T: QueryData,
	E: QueryError,
{
	type Data = InfiniteData<T, P>;
	type Error = E;
	type Key = K;

	fn scope_id(&self) -> ScopeId {
		self.scope.id()
	}

	fn codec(&self) -> Option<&DataCodec<InfiniteData<T, P>>> {
		self.scope.codec.as_ref()
	}
}

impl<K, P, T, E> InfiniteQueryScope<K, P, T, E>
where
	K: QueryKey,
	P: QueryData,
	T: QueryData,
	E: QueryError,
{
	/// A scope over a page fetcher, starting from `initial_page_param`.
	///
	/// Add [`with_next_page_param`](Self::with_next_page_param) to be able
	/// to load more; without it the query is a single page.
	pub fn new<F, Fut>(fetcher: F, initial_page_param: P) -> Self
	where
		F: Fn(InfiniteQueryFnContext<K, P>) -> Fut + Send + Sync + 'static,
		Fut: Future<Output = Result<T, E>> + Send + 'static,
	{
		let fetch_page: PageFetcher<K, P, T, E> = Arc::new(move |ctx| Box::pin(fetcher(ctx)));
		let params = Arc::new(PageParams {
			next: None,
			previous: None,
			max_pages: None,
		});
		let initial: InitialParamFn<K, P> = Arc::new(move |_: &K| initial_page_param.clone());
		Self {
			scope: QueryScope::from_fetcher::<F>(paged(&fetch_page, &params, &initial)),
			fetch_page,
			params,
			initial,
		}
	}

	/// Derives the next page's param from the pages so far; `None` means
	/// there is no next page.
	#[must_use]
	pub fn with_next_page_param(
		self,
		next: impl Fn(PageParamContext<'_, T, P>) -> Option<P> + Send + Sync + 'static,
	) -> Self {
		self.with_params(|params| params.next = Some(Arc::new(next)))
	}

	/// Derives the previous page's param from the pages so far; `None` means
	/// there is no previous page.
	#[must_use]
	pub fn with_previous_page_param(
		self,
		previous: impl Fn(PageParamContext<'_, T, P>) -> Option<P> + Send + Sync + 'static,
	) -> Self {
		self.with_params(|params| params.previous = Some(Arc::new(previous)))
	}

	/// Derives the first page's param from the key instead of using a fixed
	/// value.
	#[must_use]
	pub fn with_initial_page_param_fn(
		mut self,
		initial: impl Fn(&K) -> P + Send + Sync + 'static,
	) -> Self {
		self.initial = Arc::new(initial);
		self.scope.fetcher = paged(&self.fetch_page, &self.params, &self.initial);
		self
	}

	/// Keeps at most this many pages, dropping from the far end as new pages
	/// arrive. A refetch reloads only the kept pages.
	#[must_use]
	pub fn with_max_pages(self, max_pages: usize) -> Self {
		self.with_params(|params| params.max_pages = Some(max_pages.max(1)))
	}

	/// Copies the page settings, changes the copy, and points the scope's
	/// fetcher at it, so clones of the scope keep their own.
	fn with_params(mut self, change: impl FnOnce(&mut PageParams<T, P>)) -> Self {
		let mut params = PageParams::clone(&self.params);
		change(&mut params);
		self.params = Arc::new(params);
		self.scope.fetcher = paged(&self.fetch_page, &self.params, &self.initial);
		self
	}

	/// See [`QueryScope::with_name`].
	#[must_use]
	pub fn with_name(mut self, name: &str) -> Self {
		self.scope = self.scope.with_name(name);
		self
	}

	/// See [`QueryScope::with_options`].
	#[must_use]
	pub fn with_options(mut self, options: QueryOptions<E>) -> Self {
		self.scope = self.scope.with_options(options);
		self
	}

	/// See [`StaleTime`].
	#[must_use]
	pub fn with_stale_time(mut self, stale_time: impl Into<StaleTime>) -> Self {
		self.scope = self.scope.with_stale_time(stale_time);
		self
	}

	/// See [`GcTime`].
	#[must_use]
	pub fn with_gc_time(mut self, gc_time: impl Into<GcTime>) -> Self {
		self.scope = self.scope.with_gc_time(gc_time);
		self
	}

	/// See [`Retry`].
	#[must_use]
	pub fn with_retry(mut self, retry: impl Into<Retry<E>>) -> Self {
		self.scope = self.scope.with_retry(retry);
		self
	}

	/// See [`RetryDelay`].
	#[must_use]
	pub fn with_retry_delay(mut self, retry_delay: impl Into<RetryDelay<E>>) -> Self {
		self.scope = self.scope.with_retry_delay(retry_delay);
		self
	}

	/// See [`RefetchInterval`].
	#[must_use]
	pub fn with_refetch_interval(mut self, interval: impl Into<RefetchInterval>) -> Self {
		self.scope = self.scope.with_refetch_interval(interval);
		self
	}

	/// See [`QueryScope::with_tags`].
	#[must_use]
	pub fn with_tags(mut self, tags: impl IntoIterator<Item = impl Into<Tag>>) -> Self {
		self.scope = self.scope.with_tags(tags);
		self
	}

	/// See [`QueryScope::with_tags_for`].
	#[must_use]
	pub fn with_tags_for(mut self, tags: impl Fn(&K) -> Vec<Tag> + Send + Sync + 'static) -> Self {
		self.scope = self.scope.with_tags_for(tags);
		self
	}

	/// See [`QueryScope::with_ssr`].
	#[must_use]
	pub fn with_ssr(self) -> Self
	where
		T: Serialize + DeserializeOwned,
		P: Serialize + DeserializeOwned,
	{
		self.with_codec(DataCodec::json())
	}

	/// See [`QueryScope::with_codec`].
	#[must_use]
	pub fn with_codec(mut self, codec: DataCodec<InfiniteData<T, P>>) -> Self {
		self.scope = self.scope.with_codec(codec);
		self
	}

	/// See [`QueryScope::with_structural_sharing`].
	#[must_use]
	pub fn with_structural_sharing(mut self) -> Self
	where
		T: PartialEq,
		P: PartialEq,
	{
		self.scope = self.scope.with_structural_sharing();
		self
	}

	/// See [`QueryScope::with_data_eq`].
	#[must_use]
	pub fn with_data_eq(
		mut self,
		eq: impl Fn(&InfiniteData<T, P>, &InfiniteData<T, P>) -> bool + Send + Sync + 'static,
	) -> Self {
		self.scope = self.scope.with_data_eq(eq);
		self
	}

	/// The underlying scope over [`InfiniteData`].
	#[must_use]
	pub fn scope(&self) -> &QueryScope<K, InfiniteData<T, P>, E> {
		&self.scope
	}

	/// The scope's cache identity.
	///
	/// # Panics
	///
	/// See [`QueryScope::id`].
	#[must_use]
	#[track_caller]
	pub fn id(&self) -> ScopeId {
		self.scope.id()
	}
}

/// The fetcher the cache runs for an infinite scope: [`fetch_pages`] over
/// one set of page settings.
fn paged<K, P, T, E>(
	fetch_page: &PageFetcher<K, P, T, E>,
	params: &Arc<PageParams<T, P>>,
	initial: &InitialParamFn<K, P>,
) -> Fetcher<K, InfiniteData<T, P>, E>
where
	K: QueryKey,
	P: QueryData,
	T: QueryData,
	E: QueryError,
{
	let (fetch_page, params, initial) = (
		Arc::clone(fetch_page),
		Arc::clone(params),
		Arc::clone(initial),
	);
	Arc::new(move |ctx: FetchContext<K, InfiniteData<T, P>>| {
		Box::pin(fetch_pages(
			Arc::clone(&fetch_page),
			Arc::clone(&params),
			Arc::clone(&initial),
			ctx,
		)) as BoxFuture<'static, Result<InfiniteData<T, P>, E>>
	})
}

/// TanStack's `infiniteQueryBehavior`: extend one end, or (re)load pages from
/// the first.
async fn fetch_pages<K, P, T, E>(
	fetcher: PageFetcher<K, P, T, E>,
	params: Arc<PageParams<T, P>>,
	initial: InitialParamFn<K, P>,
	ctx: FetchContext<K, InfiniteData<T, P>>,
) -> Result<InfiniteData<T, P>, E>
where
	K: QueryKey,
	P: QueryData,
	T: QueryData,
	E: QueryError,
{
	let current = ctx.current.unwrap_or_default();
	let fetch_page = |param: P, direction: Option<FetchDirection>| {
		fetcher(InfiniteQueryFnContext {
			key: ctx.key.clone(),
			page_param: param,
			direction,
			signal: ctx.signal.clone(),
		})
	};

	if let Some(direction) = ctx.direction.filter(|_| !current.is_empty()) {
		let backward = direction == FetchDirection::Backward;
		let param = if backward {
			params.previous_param(&current)
		} else {
			params.next_param(&current)
		};
		let Some(param) = param else {
			return Ok(current);
		};
		let page = fetch_page(param.clone(), Some(direction)).await?;
		let mut data = current;
		params.add_page(&mut data, page, param, backward);
		return Ok(data);
	}

	// First load, or a refetch that reloads as many pages as were loaded,
	// starting from the first page's own param and following cursors afresh.
	let remaining = ctx.pages.unwrap_or(current.len()).max(1);
	let mut data = InfiniteData::default();
	let mut index = 0;
	loop {
		let param = if index == 0 {
			current
				.page_params
				.first()
				.cloned()
				.unwrap_or_else(|| initial(&ctx.key))
		} else {
			match params.next_param(&data) {
				Some(param) => param,
				None => break,
			}
		};
		let page = fetch_page(param.clone(), None).await?;
		params.add_page(&mut data, page, param, false);
		index += 1;
		if index >= remaining {
			break;
		}
	}
	Ok(data)
}

/// A live view of one infinite query, returned by [`use_infinite_query`].
///
/// `Copy`, like a leptos signal. Derefs to [`QueryHandle`] for everything a
/// plain query offers.
pub struct InfiniteQueryHandle<T, P, E> {
	handle: QueryHandle<InfiniteData<T, P>, E>,
	params: StoredValue<Arc<PageParams<T, P>>>,
}

impl<T, P, E> Clone for InfiniteQueryHandle<T, P, E> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<T, P, E> Copy for InfiniteQueryHandle<T, P, E> {}

impl<T, P, E> fmt::Debug for InfiniteQueryHandle<T, P, E>
where
	T: QueryData,
	P: QueryData,
	E: QueryError,
{
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("InfiniteQueryHandle")
			.field("handle", &self.handle)
			.finish_non_exhaustive()
	}
}

impl<T, P, E> Deref for InfiniteQueryHandle<T, P, E> {
	type Target = QueryHandle<InfiniteData<T, P>, E>;

	fn deref(&self) -> &Self::Target {
		&self.handle
	}
}

impl<T, P, E> InfiniteQueryHandle<T, P, E>
where
	T: QueryData,
	P: QueryData,
	E: QueryError,
{
	/// The pages loaded so far.
	pub fn pages(&self) -> Vec<T> {
		self.handle
			.data()
			.map(|data| data.pages)
			.unwrap_or_default()
	}

	/// The param each page was fetched with.
	pub fn page_params(&self) -> Vec<P> {
		self.handle
			.data()
			.map(|data| data.page_params)
			.unwrap_or_default()
	}

	/// Reads the pages without cloning them.
	pub fn with_pages<R>(&self, read: impl FnOnce(&[T]) -> R) -> Option<R> {
		self.handle.with_data(|data| read(&data.pages))
	}

	/// The items of every page flattened, for pages that are collections.
	pub fn items<I>(&self) -> Vec<I>
	where
		for<'a> &'a T: IntoIterator<Item = &'a I>,
		I: Clone,
	{
		self.handle
			.with_data(|data| data.items().cloned().collect())
			.unwrap_or_default()
	}

	/// The items of every page flattened, picked out of each page by `items`
	/// — for pages shaped like `{ items, next_cursor }`.
	pub fn items_with<I: Clone>(&self, items: impl Fn(&T) -> &[I]) -> Vec<I> {
		self.handle
			.with_data(|data| data.items_with(&items).cloned().collect())
			.unwrap_or_default()
	}

	/// Whether `with_next_page_param` finds a page after the last.
	pub fn has_next_page(&self) -> bool {
		let Some(params) = self.params.try_get_value() else {
			return false;
		};
		self.handle
			.with_data(|data| params.next_param(data).is_some())
			.unwrap_or(false)
	}

	/// Whether `with_previous_page_param` finds a page before the first.
	pub fn has_previous_page(&self) -> bool {
		let Some(params) = self.params.try_get_value() else {
			return false;
		};
		self.handle
			.with_data(|data| params.previous_param(data).is_some())
			.unwrap_or(false)
	}

	/// Loads the next page. The fetch runs whether or not the returned
	/// [`PageFetch`] is awaited; await it to wait for the page to land.
	pub fn fetch_next_page(&self) -> PageFetch {
		self.fetch_direction(FetchDirection::Forward)
	}

	/// Loads the previous page. The fetch runs whether or not the returned
	/// [`PageFetch`] is awaited; await it to wait for the page to land.
	pub fn fetch_previous_page(&self) -> PageFetch {
		self.fetch_direction(FetchDirection::Backward)
	}

	fn fetch_direction(&self, direction: FetchDirection) -> PageFetch {
		let fetch = self.handle.current_query().map(|query| {
			query.fetch(FetchOptions {
				direction: Some(direction),
				cancel_refetch: true,
				pages: None,
			})
		});
		PageFetch::new(Box::pin(async move {
			if let Some(fetch) = fetch {
				fetch.await;
			}
		}))
	}

	/// A next-page fetch is in flight.
	pub fn is_fetching_next_page(&self) -> bool {
		self.handle.is_fetching()
			&& self.handle.meta().fetch_direction == Some(FetchDirection::Forward)
	}

	/// A previous-page fetch is in flight.
	pub fn is_fetching_previous_page(&self) -> bool {
		self.handle.is_fetching()
			&& self.handle.meta().fetch_direction == Some(FetchDirection::Backward)
	}

	/// The last next-page fetch failed.
	pub fn is_fetch_next_page_error(&self) -> bool {
		self.handle.is_error()
			&& self.handle.meta().failed_direction == Some(FetchDirection::Forward)
	}

	/// The last previous-page fetch failed.
	pub fn is_fetch_previous_page_error(&self) -> bool {
		self.handle.is_error()
			&& self.handle.meta().failed_direction == Some(FetchDirection::Backward)
	}
}

/// Observes an infinite query whose key is reactive. See
/// [`use_query`](crate::use_query).
#[track_caller]
pub fn use_infinite_query<K, P, T, E, R>(
	scope: InfiniteQueryScope<K, P, T, E>,
	key: impl Fn() -> R + Send + Sync + 'static,
) -> InfiniteQueryHandle<T, P, E>
where
	K: QueryKey,
	P: QueryData,
	T: QueryData,
	E: QueryError,
	R: IntoMaybeKey<K>,
{
	use_infinite_query_with(scope, key, QueryObserverOptions::default())
}

/// [`use_infinite_query`] with per-observer options.
// Scopes are values built at the call site; taking one by value reads better
// than borrowing a temporary.
#[allow(clippy::needless_pass_by_value)]
#[track_caller]
pub fn use_infinite_query_with<K, P, T, E, R>(
	scope: InfiniteQueryScope<K, P, T, E>,
	key: impl Fn() -> R + Send + Sync + 'static,
	options: QueryObserverOptions<InfiniteData<T, P>, E>,
) -> InfiniteQueryHandle<T, P, E>
where
	K: QueryKey,
	P: QueryData,
	T: QueryData,
	E: QueryError,
	R: IntoMaybeKey<K>,
{
	let client = use_query_client();
	let handle = use_query_in(client, scope.scope.clone(), key, options);
	InfiniteQueryHandle {
		handle,
		params: StoredValue::new(Arc::clone(&scope.params)),
	}
}

impl QueryClient {
	/// [`fetch_query`](Self::fetch_query) for an infinite query: the first
	/// page, or every loaded page when stale.
	pub async fn fetch_infinite_query<K, P, T, E>(
		&self,
		scope: &InfiniteQueryScope<K, P, T, E>,
		key: K,
	) -> Result<InfiniteData<T, P>, E>
	where
		K: QueryKey,
		P: QueryData,
		T: QueryData,
		E: QueryError,
	{
		self.fetch_query(&scope.scope, key).await
	}

	/// Fetches `pages` pages in sequence, following cursors from the first.
	pub async fn fetch_infinite_query_pages<K, P, T, E>(
		&self,
		scope: &InfiniteQueryScope<K, P, T, E>,
		key: K,
		pages: usize,
	) -> Result<InfiniteData<T, P>, E>
	where
		K: QueryKey,
		P: QueryData,
		T: QueryData,
		E: QueryError,
	{
		let query = self.get_or_create(&scope.scope, key);
		let loaded = query
			.signals
			.data
			.with_untracked(|data| data.as_ref().map_or(0, InfiniteData::len));
		if loaded >= pages && !query.is_stale_for(query.options.stale_time) {
			if let Some(data) = query.signals.data.get_untracked() {
				return Ok(data);
			}
		}
		query
			.fetch(FetchOptions {
				direction: None,
				cancel_refetch: true,
				pages: Some(pages),
			})
			.await;
		Self::settle(&query).await
	}

	/// [`prefetch_query`](Self::prefetch_query) for an infinite query.
	pub async fn prefetch_infinite_query<K, P, T, E>(
		&self,
		scope: &InfiniteQueryScope<K, P, T, E>,
		key: K,
	) where
		K: QueryKey,
		P: QueryData,
		T: QueryData,
		E: QueryError,
	{
		let _ = self.fetch_infinite_query(scope, key).await;
	}

	/// [`ensure_query_data`](Self::ensure_query_data) for an infinite query.
	pub async fn ensure_infinite_query_data<K, P, T, E>(
		&self,
		scope: &InfiniteQueryScope<K, P, T, E>,
		key: K,
	) -> Result<InfiniteData<T, P>, E>
	where
		K: QueryKey,
		P: QueryData,
		T: QueryData,
		E: QueryError,
	{
		self.ensure_query_data(&scope.scope, key).await
	}

	/// [`set_query_data`](Self::set_query_data) for an infinite query.
	pub fn set_infinite_query_data<K, P, T, E>(
		&self,
		scope: &InfiniteQueryScope<K, P, T, E>,
		key: K,
		data: InfiniteData<T, P>,
	) where
		K: QueryKey,
		P: QueryData,
		T: QueryData,
		E: QueryError,
	{
		self.set_query_data(&scope.scope, key, data);
	}

	/// [`update_query_data`](Self::update_query_data) for an infinite query.
	pub fn update_infinite_query_data<K, P, T, E>(
		&self,
		scope: &InfiniteQueryScope<K, P, T, E>,
		key: K,
		updater: impl FnOnce(Option<InfiniteData<T, P>>) -> Option<InfiniteData<T, P>>,
	) -> Option<InfiniteData<T, P>>
	where
		K: QueryKey,
		P: QueryData,
		T: QueryData,
		E: QueryError,
	{
		self.update_query_data(&scope.scope, key, updater)
	}
}
