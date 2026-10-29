# leptoskit_use

Cross-platform Leptos primitives inspired by [leptos-use](https://leptos-use.rs/). The reactive core uses Leptos's `reactive_graph` directly and requires no DOM or particular executor. Optional persistence adapters supply native application files and browser local storage. Application state, hooks, and screen callbacks stay shared across renderers.

```rust
use leptos::prelude::*;
use leptoskit_use::{use_toggle, UseToggleReturn};

let UseToggleReturn { value, toggle, .. } = use_toggle(false);
toggle();
assert!(value.get());
```

Use the crate directly or enable `leptoskit`'s `use` feature and import `leptoskit::hooks::*`. Enable `leptoskit_use/local-storage` or `leptoskit/storage` for built-in persistence. Create hooks inside a Leptos `Owner`. Callback captures and stored values are `Send + Sync`; storage futures can stay local to the host event loop.

## Compatibility

This is an independent implementation. It mirrors the portable portion of leptos-use and makes environmental inputs explicit. Hooks accept Leptos signals and memos. Generic predicates, string conversion, and signal filters use readable-signal traits for type inference; wrap static inputs to those hooks in `Signal::stored`. Other derived hooks accept `Signal<T>` conversions, including static values. Read derived outputs inside a reactive context to subscribe to updates.

| leptos-use family       | Available here                                                                                        | Differences                                                                                              |
| ----------------------- | ----------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| Toggle                  | `use_toggle`, `UseToggleReturn`, `MaybeRwSignal`                                                      | Initial value or existing `RwSignal`; value, setter, toggle pattern.                                     |
| Cycle list              | `use_cycle_list`                                                                                      | Reactive `Vec`; selection follows index. Empty lists return `None`; set-index wraps.                     |
| Sorting                 | `use_sorted`, `use_sorted_by`, `use_sorted_by_key`                                                    | Stable sorted copies of vectors.                                                                         |
| Conversion / predicates | `use_to_string`, `is_some`, `is_none`, `is_ok`, `is_err`                                              | Borrow values when possible.                                                                             |
| Logic / math            | `use_not`, `use_and`, `use_or`, `use_abs`, `use_ceil`, `use_floor`, `use_round`, `use_min`, `use_max` | Unary numbers use `f64`; min/max use ordered vectors and return `None` for empty input.                  |
| Watch                   | `watch_with_options`, `watch_pausable`, `whenever`, `sync_signal`                                     | Synchronous on every host, including SSR; equality suppresses redundant callbacks.                       |
| Timers                  | `use_timeout_fn`, `use_interval_fn`, `use_interval`                                                   | Explicit monotonic clock and `Duration`; intervals coalesce missed ticks.                                |
| Filters                 | `use_debounce_fn`, `use_throttle_fn`, `signal_debounced`, `signal_throttled`                          | Trailing debounce; leading-only throttle. Explicit clock.                                                |
| Storage                 | `use_storage`, `use_async_storage`, `Storage`, `AsyncStorage`, `MemoryStorage`, `LocalStorage`        | Typed host adapters; fallible operations; persist before updating signals. Optional persistent backends. |
| Async state             | `use_async_state`, `AsyncState`                                                                       | Executor-free; each execution is a caller-driven future with pending/success/error state.                |
| Capability              | `use_supported`                                                                                       | Wraps host or plugin capability probes as signals.                                                       |
| Viewport                | `use_window_size`, `use_breakpoints`                                                                  | Host supplies integer or fractional logical dimensions. Named queries return `None` for unknown names.   |

DOM-specific hooks (selectors, CSS, HTML attributes, browser document, mutation/intersection observers) have no renderer-independent meaning and are not exported into shared code. The optional `web` feature adds a `leptoskit_use::web` module compiled only for browser targets, holding the browser-only subset: `use_event_listener`, `use_media_query`, `use_preferred_dark`, and `use_preferred_reduced_motion`. Enabling `leptoskit`'s `web` feature turns it on wherever `leptoskit::use` is enabled. Device and network capabilities such as clipboard, geolocation, and battery live in the plugin crates, several of which ship their own signal hooks.

## Host boundaries and shared Tailwind screens

The [shared component guide](../../docs/src/guide/07-shared-components.md) uses one `Element` tree on native and web. Hooks follow the same boundary: shared application logic owns signals and storage operations; the host supplies a storage backend, task scheduler, viewport, and clock.

Select persistence once at host startup. `LocalStorage<T>` names the appropriate built-in backend on native and browser targets:

```rust
// Native bootstrap: data_directory comes from the application's platform host.
let storage = leptoskit_use::LocalStorage::<Settings>::new(data_directory)?;

// Browser bootstrap: the namespace isolates this application's keys.
let storage = leptoskit_use::LocalStorage::<Settings>::new("my-app/settings")?;
```

Those are alternative host bootstraps. The shared model uses the same call in either application:

```rust
let initialize = owner.with(|| {
    leptoskit_use::use_async_storage(storage, "preferences", Settings::default())
});
let settings = initialize.await?;
(settings.set_value)(updated_preferences).await?;
```

The returned handle depends on the value and error types, so a shared screen can accept `UseAsyncStorageReturn<Settings, StorageError>` without naming its platform backend. `Arc<dyn AsyncStorage<Settings, Error = StorageError>>` also supports runtime injection. Shared button callbacks hand the returned future to the host's local executor; no executor is installed globally by the hook. Native I/O adapters use worker threads internally.

The integration tests use the exact same Tailwind settings screen and save callback through a native `NativeSurface` and the browser web renderer, then reopen persistence to verify the saved value.

## Storage behavior

`Storage<T>` defines synchronous operations. `AsyncStorage<T>` defines asynchronous operations, including futures that are not `Send`. Both are independent of the renderer. `MemoryStorage<T>` implements both contracts for headless applications, tests, and request-scoped SSR. Allocate separate memory stores per SSR request to isolate users.

The optional `local-storage` feature adds:

- `FileStorage<T>` on native: JSON files in an application directory selected by the host. Keys use stable hashed filenames, so empty, long, Unicode, and path-like keys stay within that directory. Writes atomically replace one key. Async operations offload serialization and filesystem work to a worker pool and work with any local executor.
- `WebStorage<T>` in the browser: namespaced JSON values in browser local storage. Access, quota, and decoding errors propagate. Browser local storage is synchronous, so its async operations complete immediately. For IndexedDB or another genuinely asynchronous browser store, implement `AsyncStorage<T>`; the hook accepts its local futures.

WASI, workers, and embedded hosts can implement the same traits for their available capabilities. The `leptoskit_plugin_store` and `leptoskit_plugin_secure_store` crates ship adapters behind their `async-storage` features, covering native guest settings and OS credential-store secrets. Browser local storage reports an explicit capability error in workers without a window. SSR does not probe a browser or silently choose a process-global store; its host selects request storage or an explicit persistent directory.

Both hooks load existing values and use the initial fallback for missing keys without writing it. A successful save updates the signal after persistence commits. Save, reload, or removal errors preserve the last successful value. Removal resets the signal after deleting the key. Instances do not implicitly synchronize: call `reload` from the host's change notification handler.

`use_async_storage` allocates signals when called inside the owner, before its first await. Initialization can resume outside that owner context. Operations on one handle serialize across awaits, so a pending save cannot be overtaken by its reload. Dropping an operation prevents its later signal update; backend I/O already running may still commit, so reload when cancelling a save. If the owner is disposed while I/O is pending, the operation returns `AsyncStorageError::Disposed` and does not access the dead signal. Calls on disposed async handles reject I/O.

## Driving time

One host clock can drive every timer. Wire updates to a native event loop, browser frame callback, or server scheduler. A static SSR clock schedules no background work. Its origin is arbitrary and time must be monotonic. Advancing the signal synchronously runs due callbacks on the calling thread.

```rust
use std::time::Duration;
use leptos::prelude::*;
use leptoskit_use::use_timeout_fn;

let now = RwSignal::new(Duration::ZERO);
let finished = RwSignal::new(false);
let timer = use_timeout_fn(now, move || finished.set(true), Duration::from_secs(1));
now.set(Duration::from_secs(1));
assert!(finished.get());
assert!(!timer.is_pending.get());
```

Timeouts and intervals start on creation. Zero duration fires on the next clock change. Pause/stop cancels the deadline; start/resume schedules from the current time. Owner cleanup unsubscribes watchers and timers. Arena signal handles must not be invoked after owner disposal, except watcher controls and async storage operations, which explicitly handle disposal.

Watch callbacks run untracked. Pausing continues observing source changes without invoking callbacks; resuming listens for future changes without replaying history. Avoid recursively writing a watcher's own source. `sync_signal` checks equality before writing to prevent feedback.

## Viewports

Use `Viewport<f32>` with native surface dimensions to preserve fractional logical pixels. Integer viewports continue to work. Breakpoint queries use the same units as their width signal, including half-open ranges `[lower, upper)`. Floating-point NaN matches no query. Hosts supply valid logical dimensions; the hook does not guess physical pixel density or read a global window.

## Nightly call syntax

The optional `nightly` feature mirrors Leptos's `nightly` feature. On a nightly compiler, signals from every hook read and write with call syntax (`value()` instead of `value.get()`, `set_value(true)` instead of `set_value.set(true)`), signals compose wherever closures are accepted, and `MaybeRwSignal` itself becomes callable. On stable toolchains the feature is a no-op, so `--all-features` builds keep working. See the [nightly reference](../../docs/src/reference/nightly.md) for details.

## Verification

`devenv shell coverage:use` enforces 100% line coverage for the portable and native code. `devenv shell coverage:use:browser` separately instruments the browser adapter and enforces 100% line coverage from Chrome and worker execution. It pins a nightly compiler for wasm coverage; application builds remain on the workspace toolchain. Both commands emit LCOV reports. Browser suites also run normally in Chrome and Firefox. CI checks native desktop, browser WASM, WASI, iOS, and Android compilation. Deterministic async tests suspend individual operations to exercise errors, serialization, cancellation, and owner disposal without sleeps.
