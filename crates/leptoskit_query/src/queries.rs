//! Observing a dynamic list of queries at once: [`use_queries`].

use std::fmt;
use std::sync::Arc;

use leptos::prelude::*;

use crate::client::use_query_client;
use crate::handle::QueryHandle;
use crate::handle::QueryObserverOptions;
use crate::handle::QueryResult;
use crate::observer::Observer;
use crate::scope::QueryData;
use crate::scope::QueryError;
use crate::scope::QueryKey;
use crate::scope::QueryScope;

/// A live view of a list of queries, returned by [`use_queries`].
///
/// `Copy`, like a leptos signal. Positional, like TanStack's `useQueries`:
/// the n-th handle follows the n-th key, so a stable list of keys keeps
/// stable handles.
pub struct QueriesHandle<T, E> {
	handles: RwSignal<Vec<QueryHandle<T, E>>>,
}

impl<T, E> Clone for QueriesHandle<T, E> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<T, E> Copy for QueriesHandle<T, E> {}

impl<T: 'static, E: 'static> fmt::Debug for QueriesHandle<T, E> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("QueriesHandle")
			.field("len", &self.handles.try_with_untracked(Vec::len))
			.finish()
	}
}

impl<T, E> QueriesHandle<T, E>
where
	T: QueryData,
	E: QueryError,
{
	/// The handle for each key, in key order.
	pub fn handles(&self) -> Vec<QueryHandle<T, E>> {
		self.handles.try_get().unwrap_or_default()
	}

	/// How many queries are observed.
	pub fn len(&self) -> usize {
		self.handles.try_with(Vec::len).unwrap_or(0)
	}

	/// Whether no queries are observed.
	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}

	fn with_handles<R>(&self, read: impl FnOnce(&[QueryHandle<T, E>]) -> R) -> R {
		// Handles are `Copy`, so cloning the list out is a cheap way to read
		// it even after the owner is gone.
		let handles = self.handles.try_get().unwrap_or_default();
		read(&handles)
	}

	/// Every query's result, in key order.
	pub fn get(&self) -> Vec<QueryResult<T, E>> {
		self.with_handles(|handles| handles.iter().map(QueryHandle::get).collect())
	}

	/// Every query's data, in key order; `None` where a query has none yet.
	pub fn data(&self) -> Vec<Option<T>> {
		self.with_handles(|handles| handles.iter().map(QueryHandle::data).collect())
	}

	/// Every query's data once all of them have some, TanStack's typical
	/// `combine`.
	pub fn all_data(&self) -> Option<Vec<T>> {
		self.with_handles(|handles| handles.iter().map(QueryHandle::data).collect())
	}

	/// Every error, in key order, skipping queries without one.
	pub fn errors(&self) -> Vec<E> {
		self.with_handles(|handles| handles.iter().filter_map(QueryHandle::error).collect())
	}

	/// Whether any query is still pending.
	pub fn is_pending(&self) -> bool {
		self.with_handles(|handles| handles.iter().any(QueryHandle::is_pending))
	}

	/// Whether any query is on its first load.
	pub fn is_loading(&self) -> bool {
		self.with_handles(|handles| handles.iter().any(QueryHandle::is_loading))
	}

	/// Whether any query is fetching.
	pub fn is_fetching(&self) -> bool {
		self.with_handles(|handles| handles.iter().any(QueryHandle::is_fetching))
	}

	/// Whether any query has failed.
	pub fn is_error(&self) -> bool {
		self.with_handles(|handles| handles.iter().any(QueryHandle::is_error))
	}

	/// Whether every query has data.
	pub fn is_success(&self) -> bool {
		self.with_handles(|handles| handles.iter().all(QueryHandle::is_success))
	}

	/// Refetches every query.
	pub fn refetch_all(&self) {
		for handle in self.handles() {
			handle.refetch();
		}
	}
}

/// Observes one query per key in a reactive list of keys.
///
/// Keys may come and go; handles are added, rebound, and released to match.
/// A handle whose position no longer has a key reads as disposed.
///
/// ```
/// # use leptos::prelude::*;
/// # use leptoskit_query::*;
/// # #[derive(Clone, Debug)] struct User;
/// # #[derive(Clone, Debug)] struct ApiError;
/// # async fn fetch_user(id: u32) -> Result<User, ApiError> { let _ = id; Ok(User) }
/// # fn user() -> QueryScope<u32, User, ApiError> { QueryScope::new(|id| async move { fetch_user(id).await }) }
/// #[component]
/// fn Team(member_ids: RwSignal<Vec<u32>>) -> impl IntoView {
/// 	let members = use_queries(user(), move || member_ids.get());
/// 	move || members.all_data().map(|users| users.len())
/// }
/// ```
#[track_caller]
pub fn use_queries<K, T, E>(
	scope: QueryScope<K, T, E>,
	keys: impl Fn() -> Vec<K> + Send + Sync + 'static,
) -> QueriesHandle<T, E>
where
	K: QueryKey,
	T: QueryData,
	E: QueryError,
{
	let client = use_query_client();
	let keys = ArcMemo::new(move |_| keys());
	let handles = RwSignal::new(Vec::<QueryHandle<T, E>>::new());

	// Handles belong to the component, not to the effect that adds them: an
	// effect disposes whatever its previous run owned each time it re-runs.
	let owner = Owner::current();

	let sync: Arc<dyn Fn(Vec<K>) + Send + Sync> = Arc::new(move |keys: Vec<K>| {
		let mut current = handles.try_get_untracked().unwrap_or_default();
		if current.len() > keys.len() {
			// Disposing drops the observer, which unregisters it.
			for removed in current.drain(keys.len()..) {
				removed.dispose();
			}
		}
		for (index, key) in keys.into_iter().enumerate() {
			if let Some(observer) = current.get(index).and_then(QueryHandle::observer) {
				observer.bind(&scope, Some(key));
				continue;
			}

			let observer = Observer::new(client.clone(), QueryObserverOptions::default());
			observer.bind(&scope, Some(key));
			let handle = match &owner {
				Some(owner) => owner.with(|| QueryHandle::new(observer)),
				None => QueryHandle::new(observer),
			};
			if index < current.len() {
				current[index] = handle;
			} else {
				current.push(handle);
			}
		}

		handles.try_set(current);
	});
	sync(keys.get_untracked());

	let effect_sync = Arc::clone(&sync);
	Effect::new_isomorphic(move |_| {
		effect_sync(keys.get());
		for observer in handles
			.try_get_untracked()
			.unwrap_or_default()
			.iter()
			.filter_map(QueryHandle::observer)
		{
			observer.track_rebind_sources();
		}
	});

	on_cleanup(move || {
		for observer in handles
			.try_get_untracked()
			.unwrap_or_default()
			.iter()
			.filter_map(QueryHandle::observer)
		{
			observer.unbind();
		}
	});

	QueriesHandle { handles }
}
