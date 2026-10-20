//! Query scopes: a typed fetcher plus the options every query made from it
//! shares.

use std::convert::Infallible;
use std::fmt;
use std::future::Future;
use std::hash::Hash;
use std::ops::Deref;
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::cancel::CancelToken;
use crate::key::ScopeId;
use crate::key::Tag;
use crate::options::GcTime;
use crate::options::QueryOptions;
use crate::options::RefetchInterval;
use crate::options::StaleTime;
use crate::retry::Retry;
use crate::retry::RetryDelay;
use crate::state::FetchDirection;
use crate::time::BoxFuture;

/// What a query key needs: hashable, comparable, cloneable, printable, and
/// shareable across threads. Implemented for every such type.
pub trait QueryKey: Hash + Eq + Clone + fmt::Debug + Send + Sync + 'static {}

impl<K> QueryKey for K where K: Hash + Eq + Clone + fmt::Debug + Send + Sync + 'static {}

/// What query data needs: cloneable and shareable across threads.
/// Implemented for every such type.
pub trait QueryData: Clone + Send + Sync + 'static {}

impl<T> QueryData for T where T: Clone + Send + Sync + 'static {}

/// What a query error needs: cloneable and shareable across threads.
/// Implemented for every such type.
pub trait QueryError: Clone + Send + Sync + 'static {}

impl<E> QueryError for E where E: Clone + Send + Sync + 'static {}

/// Anything that names a scope and its types: [`QueryScope`] and
/// [`InfiniteQueryScope`](crate::InfiniteQueryScope).
///
/// Client methods that only read the cache take any `ScopeLike`, so the same
/// `get_query_data` serves plain and infinite queries.
pub trait ScopeLike {
	/// The key type.
	type Key: QueryKey;
	/// The cached data type.
	type Data: QueryData;
	/// The error type.
	type Error: QueryError;

	/// The scope's cache identity.
	fn scope_id(&self) -> ScopeId;

	/// The codec that serializes this scope's data, if it has one.
	fn codec(&self) -> Option<&DataCodec<Self::Data>>;
}

impl<K, T, E> ScopeLike for QueryScope<K, T, E>
where
	K: QueryKey,
	T: QueryData,
	E: QueryError,
{
	type Data = T;
	type Error = E;
	type Key = K;

	fn scope_id(&self) -> ScopeId {
		self.id
	}

	fn codec(&self) -> Option<&DataCodec<T>> {
		self.codec.as_ref()
	}
}

/// What a query function receives when it needs more than the key.
///
/// Derefs to the key, so `ctx.id` reads a field of the key directly.
#[derive(Clone, Debug)]
pub struct QueryFnContext<K> {
	/// The key being fetched.
	pub key: K,
	/// Cancelled when the fetch is cancelled, so long-running work can stop.
	pub signal: CancelToken,
}

impl<K> Deref for QueryFnContext<K> {
	type Target = K;

	fn deref(&self) -> &K {
		&self.key
	}
}

/// The full context the cache hands a fetch behaviour.
///
/// Plain scopes only look at the key; infinite scopes also use the current
/// data and the direction to decide which page to fetch.
pub(crate) struct FetchContext<K, T> {
	pub key: K,
	pub direction: Option<FetchDirection>,
	pub current: Option<T>,
	pub signal: CancelToken,
	/// For infinite queries fetched imperatively: how many pages to load.
	pub pages: Option<usize>,
}

pub(crate) type Fetcher<K, T, E> =
	Arc<dyn Fn(FetchContext<K, T>) -> BoxFuture<'static, Result<T, E>> + Send + Sync>;
type EncodeFn<T> = Arc<dyn Fn(&T) -> Result<serde_json::Value, serde_json::Error> + Send + Sync>;
type DecodeFn<T> = Arc<dyn Fn(serde_json::Value) -> Result<T, serde_json::Error> + Send + Sync>;
type TagsFn<K> = Arc<dyn Fn(&K) -> Vec<Tag> + Send + Sync>;
pub(crate) type DataEqFn<T> = Arc<dyn Fn(&T, &T) -> bool + Send + Sync>;

/// Encodes query data for SSR streaming and persisted caches.
pub struct DataCodec<T> {
	encode: EncodeFn<T>,
	decode: DecodeFn<T>,
}

impl<T> Clone for DataCodec<T> {
	fn clone(&self) -> Self {
		Self {
			encode: Arc::clone(&self.encode),
			decode: Arc::clone(&self.decode),
		}
	}
}

impl<T: 'static> DataCodec<T> {
	/// A codec from a pair of functions.
	pub fn new(
		encode: impl Fn(&T) -> Result<serde_json::Value, serde_json::Error> + Send + Sync + 'static,
		decode: impl Fn(serde_json::Value) -> Result<T, serde_json::Error> + Send + Sync + 'static,
	) -> Self {
		Self {
			encode: Arc::new(encode),
			decode: Arc::new(decode),
		}
	}

	/// The serde JSON codec.
	#[must_use]
	pub fn json() -> Self
	where
		T: Serialize + DeserializeOwned,
	{
		Self::new(
			|value: &T| serde_json::to_value(value),
			serde_json::from_value,
		)
	}

	/// Encodes a value.
	pub fn encode(&self, value: &T) -> Result<serde_json::Value, serde_json::Error> {
		(self.encode)(value)
	}

	/// Decodes a value.
	pub fn decode(&self, value: serde_json::Value) -> Result<T, serde_json::Error> {
		(self.decode)(value)
	}
}

impl<T> fmt::Debug for DataCodec<T> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("DataCodec(..)")
	}
}

/// A typed fetcher and the options its queries share.
///
/// A scope is the Rust answer to TanStack Query's `queryKey[0]`. Where
/// TanStack would write `useQuery({ queryKey: ['todo', id], queryFn })`, here
/// the fetcher *is* the scope and the key is just `id`:
///
/// ```
/// use leptoskit_query::QueryScope;
///
/// # #[derive(Clone, Debug)] struct Todo;
/// # #[derive(Clone, Debug)] struct ApiError;
/// # async fn fetch_todo(id: u32) -> Result<Todo, ApiError> { let _ = id; Ok(Todo) }
/// fn todo() -> QueryScope<u32, Todo, ApiError> {
/// 	QueryScope::new(|id| async move { fetch_todo(id).await })
/// 		.with_stale_time(std::time::Duration::from_secs(30))
/// }
/// ```
///
/// Because a scope's identity comes from the fetcher's type, calling `todo()`
/// anywhere in the app names the same cache entries. Everything the fetch
/// depends on belongs in the key, exactly as TanStack asks of query keys.
pub struct QueryScope<K, T, E> {
	pub(crate) id: ScopeId,
	pub(crate) name: Arc<str>,
	pub(crate) fetcher: Fetcher<K, T, E>,
	pub(crate) options: QueryOptions<E>,
	pub(crate) tags: Option<TagsFn<K>>,
	pub(crate) codec: Option<DataCodec<T>>,
	pub(crate) data_eq: Option<DataEqFn<T>>,
}

impl<K, T, E> Clone for QueryScope<K, T, E> {
	fn clone(&self) -> Self {
		Self {
			id: self.id,
			name: Arc::clone(&self.name),
			fetcher: Arc::clone(&self.fetcher),
			options: self.options.clone(),
			tags: self.tags.clone(),
			codec: self.codec.clone(),
			data_eq: self.data_eq.clone(),
		}
	}
}

impl<K, T, E> fmt::Debug for QueryScope<K, T, E> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("QueryScope")
			.field("id", &self.id)
			.field("name", &self.name)
			.field("options", &self.options)
			.field("ssr", &self.codec.is_some())
			.finish_non_exhaustive()
	}
}

impl<K, T, E> QueryScope<K, T, E>
where
	K: QueryKey,
	T: QueryData,
	E: QueryError,
{
	/// A scope over a fetcher that receives the key.
	///
	/// The fetcher runs once per attempt, so retries call it again with a
	/// clone of the key.
	pub fn new<F, Fut>(fetcher: F) -> Self
	where
		F: Fn(K) -> Fut + Send + Sync + 'static,
		Fut: Future<Output = Result<T, E>> + Send + 'static,
	{
		let fetcher = Arc::new(fetcher);
		Self::from_fetcher(
			ScopeId::of::<F, K, T, E>(),
			std::any::type_name::<F>(),
			Arc::new(move |ctx: FetchContext<K, T>| {
				let fut = fetcher(ctx.key);
				Box::pin(fut) as BoxFuture<'static, Result<T, E>>
			}),
		)
	}

	/// A scope over a fetcher that also receives a [`QueryFnContext`], for
	/// fetchers that want to observe cancellation.
	pub fn new_with_context<F, Fut>(fetcher: F) -> Self
	where
		F: Fn(QueryFnContext<K>) -> Fut + Send + Sync + 'static,
		Fut: Future<Output = Result<T, E>> + Send + 'static,
	{
		let fetcher = Arc::new(fetcher);
		Self::from_fetcher(
			ScopeId::of::<F, K, T, E>(),
			std::any::type_name::<F>(),
			Arc::new(move |ctx: FetchContext<K, T>| {
				let fut = fetcher(QueryFnContext {
					key: ctx.key,
					signal: ctx.signal,
				});
				Box::pin(fut) as BoxFuture<'static, Result<T, E>>
			}),
		)
	}

	pub(crate) fn from_fetcher(id: ScopeId, name: &str, fetcher: Fetcher<K, T, E>) -> Self {
		Self {
			id,
			name: Arc::from(name),
			fetcher,
			options: QueryOptions::default(),
			tags: None,
			codec: None,
			data_eq: None,
		}
	}

	/// Gives the scope a stable identity and a readable name.
	///
	/// Two scopes with the same name share cache entries, whatever their
	/// fetchers. Name scopes whose data is persisted across builds.
	#[must_use]
	pub fn with_name(mut self, name: &str) -> Self {
		self.id = ScopeId::named(name);
		self.name = Arc::from(name);
		self
	}

	/// Replaces the scope's options.
	#[must_use]
	pub fn with_options(mut self, options: QueryOptions<E>) -> Self {
		self.options = options;
		self
	}

	/// See [`StaleTime`].
	#[must_use]
	pub fn with_stale_time(mut self, stale_time: impl Into<StaleTime>) -> Self {
		self.options.stale_time = Some(stale_time.into());
		self
	}

	/// See [`GcTime`].
	#[must_use]
	pub fn with_gc_time(mut self, gc_time: impl Into<GcTime>) -> Self {
		self.options.gc_time = Some(gc_time.into());
		self
	}

	/// See [`Retry`].
	#[must_use]
	pub fn with_retry(mut self, retry: impl Into<Retry<E>>) -> Self {
		self.options.retry = Some(retry.into());
		self
	}

	/// See [`RetryDelay`].
	#[must_use]
	pub fn with_retry_delay(mut self, retry_delay: impl Into<RetryDelay<E>>) -> Self {
		self.options.retry_delay = Some(retry_delay.into());
		self
	}

	/// See [`RefetchInterval`].
	#[must_use]
	pub fn with_refetch_interval(mut self, interval: impl Into<RefetchInterval>) -> Self {
		self.options.refetch_interval = Some(interval.into());
		self
	}

	/// Tags every key with the same set of [`Tag`]s.
	#[must_use]
	pub fn with_tags(self, tags: impl IntoIterator<Item = impl Into<Tag>>) -> Self {
		let tags: Vec<Tag> = tags.into_iter().map(Into::into).collect();
		self.with_tags_for(move |_| tags.clone())
	}

	/// Derives each key's [`Tag`]s from the key, so that invalidating
	/// `tag!["users", 7]` reaches every query about that user.
	#[must_use]
	pub fn with_tags_for(mut self, tags: impl Fn(&K) -> Vec<Tag> + Send + Sync + 'static) -> Self {
		self.tags = Some(Arc::new(tags));
		self
	}

	/// Streams server-fetched data to the browser and lets the cache be
	/// dehydrated, using serde JSON.
	///
	/// Without this a query is fetched again in the browser after hydration.
	#[must_use]
	pub fn with_ssr(self) -> Self
	where
		T: Serialize + DeserializeOwned,
	{
		self.with_codec(DataCodec::json())
	}

	/// Like [`with_ssr`](Self::with_ssr) with a custom codec.
	#[must_use]
	pub fn with_codec(mut self, codec: DataCodec<T>) -> Self {
		self.codec = Some(codec);
		self
	}

	/// Keeps the existing data, and so does not notify data subscribers, when
	/// a refetch returns an equal value. TanStack calls this structural
	/// sharing.
	#[must_use]
	pub fn with_structural_sharing(self) -> Self
	where
		T: PartialEq,
	{
		self.with_data_eq(|old, new| old == new)
	}

	/// Like [`with_structural_sharing`](Self::with_structural_sharing) with a
	/// custom equality.
	#[must_use]
	pub fn with_data_eq(mut self, eq: impl Fn(&T, &T) -> bool + Send + Sync + 'static) -> Self {
		self.data_eq = Some(Arc::new(eq));
		self
	}

	/// The scope's cache identity.
	#[must_use]
	pub fn id(&self) -> ScopeId {
		self.id
	}

	/// The scope's readable name.
	#[must_use]
	pub fn name(&self) -> &str {
		&self.name
	}

	/// The scope's options.
	#[must_use]
	pub fn options(&self) -> &QueryOptions<E> {
		&self.options
	}

	/// The tags for one key.
	#[must_use]
	pub fn tags_for(&self, key: &K) -> Vec<Tag> {
		self.tags.as_ref().map_or_else(Vec::new, |tags| tags(key))
	}
}

impl<K, T> QueryScope<K, T, Infallible>
where
	K: QueryKey,
	T: QueryData,
{
	/// A scope over a fetcher that cannot fail.
	pub fn infallible<F, Fut>(fetcher: F) -> Self
	where
		F: Fn(K) -> Fut + Send + Sync + 'static,
		Fut: Future<Output = T> + Send + 'static,
	{
		let fetcher = Arc::new(fetcher);
		Self::from_fetcher(
			ScopeId::of::<F, K, T, Infallible>(),
			std::any::type_name::<F>(),
			Arc::new(move |ctx: FetchContext<K, T>| {
				let fut = fetcher(ctx.key);
				Box::pin(async move { Ok(fut.await) }) as BoxFuture<'static, Result<T, Infallible>>
			}),
		)
	}
}
