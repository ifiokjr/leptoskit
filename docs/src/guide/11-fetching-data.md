# Fetching data

`leptoskit_query` is TanStack Query for leptos: a cache of async state shared by every component that reads it, kept fresh in the background, and exposed through fine-grained signals. It lives in the `leptoskit_query` crate and behind the `query` feature of the `leptoskit` meta crate.

```toml
[dependencies]
leptoskit_query = "0.1"
```

## Provide a client

One `QueryClient` holds every query and mutation. Provide it at the root; it also wires the cache to window focus and network events in the browser.

```rust
use leptos::prelude::*;
use leptoskit_query::prelude::*;

#[component]
pub fn App() -> impl IntoView {
	QueryClient::new().provide();
	// ...
}
```

## Define a scope

Where TanStack identifies a query with an array key like `['todo', id]`, here the first part — _which fetcher_ — is a typed `QueryScope`, and the rest is a typed key. Define scopes as plain functions; a scope's identity comes from its fetcher, so every call names the same cache entries.

```rust
use std::time::Duration;

use leptoskit_query::prelude::*;

fn todo() -> QueryScope<u32, Todo, ApiError> {
	QueryScope::new(|id| async move { fetch_todo(id).await })
		.with_stale_time(Duration::from_secs(30))
		.with_tags_for(|id| vec![tag!["todos", id]])
}
```

Everything the fetch depends on belongs in the key, exactly as TanStack asks of query keys. A fetcher over a server function is the common case:

```rust
fn todo() -> QueryScope<u32, Todo, ServerFnError> {
	QueryScope::new(|id| get_todo(id))
}
```

## Read it in a component

`use_query` takes the scope and a reactive key. The handle it returns is `Copy`, like a signal, and every accessor is a reactive read.

```rust
#[component]
fn TodoView(id: RwSignal<u32>) -> impl IntoView {
	let todo = use_query(todo(), move || id.get());
	view! {
		<Show when=move || todo.is_loading()>"Loading…"</Show>
		<Show when=move || todo.is_error()>"Something went wrong"</Show>
		<p>{move || todo.data().map(|todo| todo.title)}</p>
		<button on:click=move |_| {
			todo.refetch();
		}>"Refresh"</button>
	}
}
```

A key closure may return `Option<K>`: `None` disables the query until the key is known, which is how dependent queries are written. `use_query_with` adds per-observer options such as `keep_previous_data()` for pagination, `with_enabled`, placeholder and initial data, or a different stale time.

## Write with a mutation

A `MutationScope` is `useMutation`'s options. `on_mutate` runs first and whatever it returns is the _context_ every later callback receives — the place to remember what an optimistic update should roll back to.

```rust
fn add_todo() -> MutationScope<String, Todo, ApiError, Option<Vec<Todo>>> {
	MutationScope::new(|title: String| async move { create_todo(title).await })
		.with_on_mutate_sync(|args| {
			let previous = args.client.get_query_data(&todos(), &());
			args.client.update_query_data(&todos(), (), |list| {
				let mut list = list.unwrap_or_default();
				list.push(Todo::pending(&args.variables));
				Some(list)
			});
			previous
		})
		.with_on_error(|args| {
			if let Some(previous) = args.context {
				args.client.set_query_data(&todos(), (), previous);
			}
		})
		.with_on_settled_async(|args| {
			async move {
				args.client.invalidate_scope(&todos()).await;
			}
		})
}
```

`use_mutation(add_todo())` returns a handle with `mutate`, `mutate_async`, `is_pending`, `error`, and the rest. An awaited `on_settled` keeps the mutation pending until the refetch it triggers has landed, so a form can stay disabled until the list is really up to date.

## Infinite queries

An `InfiniteQueryScope` pages in either direction. The fetcher receives the page param; `with_next_page_param` derives the next one from the pages so far.

```rust
fn feed() -> InfiniteQueryScope<UserId, Cursor, FeedPage, ApiError> {
	InfiniteQueryScope::new(
		|ctx| async move { fetch_feed(ctx.key, ctx.page_param).await },
		Cursor::start(),
	)
	.with_next_page_param(|ctx| ctx.page.next_cursor.clone())
	.with_max_pages(5)
}
```

`use_infinite_query` returns a handle with `pages`, `items_with(|page| &page.posts)`, `has_next_page`, `fetch_next_page`, and `is_fetching_next_page`, plus everything a plain query offers.

## Invalidate

`args.client.invalidate_scope(&todos())` marks every todo query stale and refetches the ones on screen. Invalidate one key with `invalidate_query`, a hierarchy with `invalidate_tag(tag!["users", 7])`, or anything a `QueryFilter` can describe — by scope, by key predicate, by staleness, by fetch status.

## Server rendering

A plain query does not fetch during SSR; the browser fetches after hydration. Add `.with_ssr()` to a scope whose data is serializable and the server fetches it, streams it with the HTML, and the browser hydrates the cache instead of fetching again. Read such queries inside `<Suspense>` with `query.await`, exactly as you would a leptos resource, so the server waits for the data before rendering the boundary.

## Across tabs

```rust
let client = QueryClient::new();
client.provide();
let _sync = client.broadcast("my-app");
```

Every tab that joins the same channel shares cache writes: log in on one tab and the others have the user. Only scopes with `.with_ssr()` take part, since the data has to be serialized. Outside the browser, `LocalBroadcast` syncs clients in one process and `BroadcastTransport` adapts anything else.

## Native and embedded hosts

The library never assumes a browser. Where it runs decides who provides time and events:

- **Browser** — the default `browser` feature uses `setTimeout`, `Date.now()`, and the document's visibility and connectivity events.
- **Server and native** — tokio (or a helper thread) and `SystemTime` cover time; the shell reports focus and connectivity with `FocusManager::set_focused` and `OnlineManager::set_online`, and calls `client.mount()` if it does not use `provide()`.
- **Wasm hosted outside a browser** — build with `--no-default-features`, then `install_clock` and `install_sleep` before creating a client. Everything else is unchanged.
