//! Async state management for leptos in the style of TanStack Query.
//!
//! Queries are cached, deduplicated, retried, refetched when stale, and
//! garbage collected when nobody is looking. Mutations get lifecycle
//! callbacks and optimistic updates. Infinite queries page in either
//! direction. Everything is exposed through fine-grained leptos signals, so a
//! view that reads `todos.data()` re-renders when the data changes and not
//! when a background refetch starts.
//!
//! # The shape of it
//!
//! Where TanStack identifies a query by an array key like `['todo', id]`, here
//! the first element is a typed [`QueryScope`] — the fetcher plus its options
//! — and the rest is a typed key:
//!
//! ```
//! use std::time::Duration;
//!
//! use leptos::prelude::*;
//! use leptoskit_query::prelude::*;
//!
//! # #[derive(Clone, Debug, PartialEq)] struct Todo { id: u32, title: String }
//! # #[derive(Clone, Debug)] struct ApiError;
//! # async fn fetch_todo(id: u32) -> Result<Todo, ApiError> { Ok(Todo { id, title: String::new() }) }
//! # async fn rename_todo(id: u32, title: String) -> Result<Todo, ApiError> { Ok(Todo { id, title }) }
//! // Scopes are plain functions: calling `todo()` anywhere names the same
//! // cache entries, because a scope's identity comes from its fetcher.
//! fn todo() -> QueryScope<u32, Todo, ApiError> {
//! 	QueryScope::new(|id| async move { fetch_todo(id).await })
//! 		.with_stale_time(Duration::from_secs(30))
//! 		.with_tags_for(|id| vec![tag!["todos", id]])
//! }
//!
//! fn rename() -> MutationScope<(u32, String), Todo, ApiError> {
//! 	MutationScope::new(|(id, title)| async move { rename_todo(id, title).await })
//! 		.with_on_success(|args| {
//! 			// Put the server's answer straight into the cache…
//! 			args.client.set_query_data(&todo(), args.data.id, args.data.clone());
//! 			// …and mark everything else about this todo stale.
//! 			args.client.invalidate_tag(tag!["todos", args.data.id]);
//! 		})
//! }
//!
//! #[component]
//! fn App() -> impl IntoView {
//! 	QueryClient::new().provide();
//! 	view! { <TodoView id=RwSignal::new(1) /> }
//! }
//!
//! #[component]
//! fn TodoView(id: RwSignal<u32>) -> impl IntoView {
//! 	let todo = use_query(todo(), move || id.get());
//! 	let rename = use_mutation(rename());
//! 	view! {
//! 		<Show when=move || todo.is_loading()>"Loading…"</Show>
//! 		<p>{move || todo.data().map(|todo| todo.title)}</p>
//! 		<button
//! 			disabled=move || rename.is_pending()
//! 			on:click=move |_| rename.mutate((id.get(), "Done".to_owned()))
//! 		>
//! 			"Rename"
//! 		</button>
//! 	}
//! }
//! ```
//!
//! # Feature map
//!
//! | TanStack Query                            | Here                                                                        |
//! | ----------------------------------------- | --------------------------------------------------------------------------- |
//! | `useQuery`                                | [`use_query`], [`use_query_with`], [`QueryHandle`]                          |
//! | `useQueries`                              | [`use_queries`]                                                             |
//! | `useInfiniteQuery`                        | [`use_infinite_query`], [`InfiniteQueryScope`], [`InfiniteQueryHandle`]     |
//! | `useMutation`                             | [`use_mutation`], [`MutationScope`], [`MutationHandle`]                     |
//! | `useSuspenseQuery`                        | `query.await` inside [`Suspend`](leptos::prelude::Suspend)                  |
//! | `useIsFetching` / `useIsMutating`         | [`use_is_fetching`], [`use_is_mutating`]                                    |
//! | `useMutationState`                        | [`use_mutation_state`]                                                      |
//! | `queryKey` partial matching               | [`QueryScope`] + typed key, [`Tag`] prefixes, [`QueryFilter`]               |
//! | `enabled`, `select`, `placeholderData`    | [`QueryObserverOptions`], [`QueryHandle::select`], [`Placeholder`]          |
//! | `staleTime`, `gcTime`, `retry`, …         | [`QueryOptions`], [`QueryDefaults`]                                         |
//! | `refetchOnWindowFocus` / `Reconnect`      | [`FocusManager`], [`OnlineManager`]                                         |
//! | `networkMode`                             | [`NetworkMode`]                                                             |
//! | `QueryClient` methods                     | [`QueryClient`]                                                             |
//! | `dehydrate` / `hydrate`                   | [`QueryClient::dehydrate`], [`QueryClient::hydrate`], [`QueryScope::with_ssr`] |
//! | `broadcastQueryClient`                    | [`QueryClient::broadcast`], [`BroadcastTransport`]                          |
//! | `AbortSignal`                             | [`CancelToken`] via [`QueryScope::new_with_context`]                        |
//! | `scope` (serial mutations)                | [`MutationScope::with_serial_scope`]                                        |
//! | `structuralSharing`                       | [`QueryScope::with_structural_sharing`]                                     |
//! | `notifyOnChangeProps`                     | Not needed: every accessor is its own signal                                |
//!
//! # Server rendering
//!
//! A plain query does not fetch during SSR; the browser fetches after
//! hydration, as TanStack does without prefetching. Add
//! [`with_ssr`](QueryScope::with_ssr) to a scope whose data is
//! serializable and the server fetches it, streams it with the HTML, and the
//! browser hydrates the cache with it — the same mechanism leptos resources
//! use — so the first render shows the server's data with no loading state.
//! Read such queries inside a `<Suspense>` with `query.await` so the server
//! waits for the data before rendering.
//!
//! Give such scopes a [stale time](QueryScope::with_stale_time). Data is
//! stale the moment it is fetched by default, so without one the browser
//! refetches in the background as soon as it hydrates, exactly as TanStack
//! does. Scopes dehydrated by hand with [`QueryClient::dehydrate`] must also
//! be [named](QueryScope::with_name), since the browser is another binary.
//!
//! # Running anywhere
//!
//! Nothing here assumes a browser. The library asks its host for four
//! things, each with a default where one makes sense:
//!
//! | Need                 | Browser (`browser` feature, on by default)         | Server / native                 | Embedded wasm (`--no-default-features`)                  |
//! | -------------------- | -------------------------------------------------- | ------------------------------- | -------------------------------------------------------- |
//! | An async executor    | `any_spawner::Executor::init_wasm_bindgen()`       | `Executor::init_tokio()`        | `Executor::init_custom_executor(..)`                     |
//! | A clock and a timer  | `Date.now()`, `setTimeout`                         | `SystemTime`, tokio or a thread | [`install_clock`], [`install_sleep`]; the `guest` feature reads leptoskit's native runtime clock |
//! | Focus and network    | `visibilitychange`, `online`/`offline` listeners   | [`FocusManager::set_focused`], [`OnlineManager::set_online`] | the same setters                |
//! | Sync between windows | `BroadcastChannel` via [`QueryClient::broadcast`]  | [`LocalBroadcast`] or a custom [`BroadcastTransport`] | a custom [`BroadcastTransport`] |
//!
//! Leptos itself installs the executor in the browser and on the server. A
//! native shell — iOS, Android, desktop — that hosts the wasm bundle outside
//! a browser builds this crate with `--no-default-features`, installs a
//! clock and a timer, and reports focus and connectivity through the
//! managers; everything else is the same code. A guest of leptoskit's own
//! native runtime turns on `guest` instead of installing a clock: that
//! runtime reports the time before every call into the guest. It has no
//! timer yet, so sleeps there still resolve at once.

// Denied rather than forbidden: the `guest` feature needs one unmangled
// export, which is the only exception.
#![deny(unsafe_code)]
// `TanStack` and `React Query` are product names, not code identifiers.
#![allow(clippy::doc_markdown)]

mod awaitable;
mod broadcast;
mod cache;
mod cancel;
mod client;
#[cfg(feature = "devtools")]
pub mod devtools;
mod filters;

mod handle;
mod hydration;
mod infinite;
mod key;
mod listeners;
#[cfg(feature = "db")]
pub mod live;
mod managers;

mod mutation;
mod mutation_cache;
mod observer;
mod options;
mod queries;
mod query;
mod retry;
mod scope;
mod state;
mod time;

pub use awaitable::PageFetch;
pub use awaitable::Refetch;
pub use awaitable::Refetches;
pub use broadcast::Broadcast;
pub use broadcast::BroadcastMessage;
pub use broadcast::BroadcastTransport;
pub use broadcast::LocalBroadcast;
pub use broadcast::LocalPeer;
pub use cache::CacheEvent;
pub use cache::CacheNotification;
pub use cancel::CancelToken;
pub use cancel::Cancelled;
pub use client::QueryClient;
pub use client::QueryClientProvider;
pub use client::use_query_client;
pub use filters::InvalidateOptions;
pub use filters::QueryFilter;
pub use filters::QueryKind;
pub use filters::RefetchOptions;
pub use filters::RefetchType;
pub use handle::IntoMaybeKey;
pub use handle::Placeholder;
pub use handle::QueryHandle;
pub use handle::QueryObserverOptions;
pub use handle::QueryResult;
pub use handle::use_is_fetching;
pub use handle::use_query;
pub use handle::use_query_in;
pub use handle::use_query_with;
pub use hydration::DehydratedQuery;
pub use hydration::DehydratedState;
pub use hydration::is_server;
pub use infinite::InfiniteData;
pub use infinite::InfiniteQueryFnContext;
pub use infinite::InfiniteQueryHandle;
pub use infinite::InfiniteQueryScope;
pub use infinite::PageParamContext;
pub use infinite::use_infinite_query;
pub use infinite::use_infinite_query_with;
pub use key::ScopeId;
pub use key::Tag;
pub use key::hash_key;
pub use listeners::Subscription;
pub use managers::FocusManager;
pub use managers::OnlineManager;
pub use managers::install_browser_listeners;
pub use mutation::ErrorArgs;
pub use mutation::MutateArgs;
pub use mutation::MutateCallbacks;
pub use mutation::MutationHandle;
pub use mutation::MutationScope;
pub use mutation::MutationState;
pub use mutation::MutationStatus;
pub use mutation::SettledArgs;
pub use mutation::SuccessArgs;
pub use mutation::use_mutation;
pub use mutation::use_mutation_in;
pub use mutation_cache::MutationEvent;
pub use mutation_cache::MutationFilter;
pub use mutation_cache::MutationInfo;
pub use mutation_cache::MutationNotification;
pub use mutation_cache::use_is_mutating;
pub use mutation_cache::use_mutation_state;
pub use options::GcTime;
pub use options::NetworkMode;
pub use options::QueryDefaults;
pub use options::QueryOptions;
pub use options::RefetchInterval;
pub use options::RefetchOn;
pub use options::ResolvedQueryOptions;
pub use options::StaleTime;
pub use queries::QueriesHandle;
pub use queries::use_queries;
pub use query::FetchFuture;
pub use query::FetchOptions;
pub use query::QueryInfo;
pub use query::QuerySignals;
pub use retry::Backoff;
pub use retry::Retry;
pub use retry::RetryDelay;
pub use retry::RetryPolicy;
pub use scope::DataCodec;
pub use scope::QueryData;
pub use scope::QueryError;
pub use scope::QueryFnContext;
pub use scope::QueryKey;
pub use scope::QueryScope;
pub use scope::ScopeLike;
pub use state::FetchDirection;
pub use state::FetchStatus;
pub use state::QueryMeta;
pub use state::QueryState;
pub use state::QueryStatus;
pub use time::AlreadyInstalled;
pub use time::BoxFuture;
pub use time::install_clock;
pub use time::install_sleep;
pub use time::now_ms;
pub use time::sleep;

/// The everything-import for components.
pub mod prelude {
	pub use crate::InfiniteQueryHandle;
	pub use crate::InfiniteQueryScope;
	pub use crate::MutateCallbacks;
	pub use crate::MutationHandle;
	pub use crate::MutationScope;
	pub use crate::NetworkMode;
	pub use crate::QueryClient;
	pub use crate::QueryClientProvider;
	pub use crate::QueryFilter;
	pub use crate::QueryHandle;
	pub use crate::QueryObserverOptions;
	pub use crate::QueryOptions;
	pub use crate::QueryScope;
	pub use crate::RefetchOn;
	pub use crate::Retry;
	pub use crate::StaleTime;
	pub use crate::Tag;
	pub use crate::tag;
	pub use crate::use_infinite_query;
	pub use crate::use_infinite_query_with;
	pub use crate::use_is_fetching;
	pub use crate::use_is_mutating;
	pub use crate::use_mutation;
	pub use crate::use_mutation_state;
	pub use crate::use_queries;
	pub use crate::use_query;
	pub use crate::use_query_client;
	pub use crate::use_query_with;
}
