# leptoskit_query

> TanStack Query for leptos: cached, deduplicated, retrying async state on fine-grained signals.

Queries are cached and shared between every component that reads them, fetched once however many ask, refetched in the background when stale, retried with backoff, paused while offline, and garbage collected when nobody is looking. Infinite queries page in either direction. Mutations get `on_mutate`/`on_success`/`on_error`/`on_settled` callbacks with a context for optimistic updates and rollback. Everything is exposed through leptos signals, so a view that reads `todos.data()` re-renders when the data changes — and not when a background refetch merely starts.

```rust
use std::time::Duration;

use leptos::prelude::*;
use leptoskit_query::prelude::*;

// A scope is TanStack's `queryKey[0]` and `queryFn` in one typed value.
fn todo() -> QueryScope<u32, Todo, ApiError> {
	QueryScope::new(|id| async move { fetch_todo(id).await })
		.with_stale_time(Duration::from_secs(30))
		.with_tags_for(|id| vec![tag!["todos", id]])
}

fn rename() -> MutationScope<(u32, String), Todo, ApiError> {
	MutationScope::new(|(id, title)| async move { rename_todo(id, title).await }).with_on_success(
		|args| {
			args.client
				.set_query_data(&todo(), args.data.id, args.data.clone());
			args.client.invalidate_tag(tag!["todos", args.data.id]);
		},
	)
}

#[component]
fn App() -> impl IntoView {
	QueryClient::new().provide();
	view! { <TodoView id=RwSignal::new(1) /> }
}

#[component]
fn TodoView(id: RwSignal<u32>) -> impl IntoView {
	let todo = use_query(todo(), move || id.get());
	let rename = use_mutation(rename());
	view! {
		<Show when=move || todo.is_loading()>"Loading…"</Show>
		<p>{move || todo.data().map(|todo| todo.title)}</p>
		<button
			disabled=move || rename.is_pending()
			on:click=move |_| rename.mutate((id.get(), "Done".to_owned()))
		>
			"Rename"
		</button>
	}
}
```

## What maps to what

| TanStack Query                                     | `leptoskit_query`                                                               |
| -------------------------------------------------- | ------------------------------------------------------------------------------- |
| `useQuery` / `useQueries` / `useInfiniteQuery`     | `use_query` / `use_queries` / `use_infinite_query`                              |
| `useMutation` / `useMutationState`                 | `use_mutation` / `use_mutation_state`                                           |
| `useSuspenseQuery`                                 | `query.await` inside `Suspend`                                                  |
| `useIsFetching` / `useIsMutating`                  | `use_is_fetching` / `use_is_mutating`                                           |
| `queryKey: ['todos', id]`, partial matching        | `QueryScope<K, T, E>` + typed key; `Tag` prefixes; `QueryFilter`                |
| `enabled`, `select`, `placeholderData`             | `QueryObserverOptions`, `handle.select`, `Placeholder::KeepPrevious`            |
| `staleTime`, `gcTime`, `retry`, `refetchInterval`… | `QueryOptions` on a scope, `QueryDefaults` on the client                        |
| `refetchOnWindowFocus` / `refetchOnReconnect`      | `FocusManager`, `OnlineManager` (browser listeners installed on `provide`)      |
| `networkMode`                                      | `NetworkMode::{Online, Always, OfflineFirst}`                                   |
| `QueryClient` methods                              | `QueryClient`: fetch/prefetch/ensure, get/set data, invalidate/refetch/cancel/… |
| `AbortSignal`                                      | `CancelToken` via `QueryScope::new_with_context`                                |
| `scope` (serial mutations)                         | `MutationScope::with_serial_scope`                                              |
| `structuralSharing`                                | `QueryScope::with_structural_sharing`                                           |
| `dehydrate` / `hydrate`                            | `client.dehydrate()` / `client.hydrate()` for scopes with `.with_ssr()`         |
| `notifyOnChangeProps`                              | not needed — every accessor is its own signal                                   |

## Server rendering

A plain query does not fetch during SSR; the browser fetches after hydration, as TanStack does without prefetching. Add `.with_ssr()` to a scope whose data is serializable and the server fetches it, streams it alongside the HTML, and the browser hydrates the cache with it — the same `SharedContext` mechanism leptos resources use — so the first render shows the server's data. Read such queries inside `<Suspense>` with `query.await` so the server waits for the data before rendering the boundary.

Give those scopes a stale time. Data is stale the moment it is fetched by default, so without one the browser refetches in the background right after hydrating, as TanStack does. Scopes dehydrated by hand with `client.dehydrate()` must also be named with `.with_name(..)`: dehydrated data is matched by scope and key, and only a name (and the stable key hash) is the same in the server's binary and the browser's.

## Across tabs and windows

`client.broadcast("my-app")` shares cache writes with every other browser tab on the same channel over a `BroadcastChannel`. The same sync layer works over anything that can carry a message: `LocalBroadcast` connects clients in one process (the windows of a native app), and implementing `BroadcastTransport` connects anything else. Only scopes with a codec (`.with_ssr()`) take part, and a peer that has not created a query yet keeps the data for up to its gc time, until something does.

## Running anywhere

Nothing here assumes a browser. The library asks its host for an async executor, a clock and a timer, focus and network status, and — optionally — a way to reach other windows. In the browser the default `browser` feature wires all of them to the DOM; on the server and native targets tokio and `SystemTime` cover time and the host reports focus and connectivity through `FocusManager::set_focused` and `OnlineManager::set_online`. A native shell hosting the wasm bundle outside a browser builds with `--no-default-features`, installs a clock and a timer with `install_clock` and `install_sleep`, and runs the very same code. A guest of leptoskit's own native runtime turns on the `guest` feature instead of installing a clock: the runtime reports the time before every call into the guest through the `leptoskit_clock` export that feature defines. That runtime has no timer yet, so sleeps there resolve at once.

## Where it differs from TanStack

- Keys are typed. A scope's identity comes from its fetcher's type, so define scopes as plain functions and put everything the fetch depends on in the key. A fetcher that captures values cannot be told apart by its type, so such a scope must be named with `.with_name(..)`; an unnamed one panics on first use rather than share a cache with every other scope built from the same closure.
- Per-call `mutate_with` callbacks fire for the call they were given, not only for the latest call.
- `fetch_query` uses the scope's retry policy rather than disabling retries; a fetch cancelled before the query has data is simply tried again.
- Handles are `Copy`, like signals: use them in as many closures as you like.
