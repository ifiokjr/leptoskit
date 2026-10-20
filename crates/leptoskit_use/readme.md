# leptoskit_use

Cross-platform Leptos primitives inspired by [leptos-use](https://leptos-use.rs/). Uses Leptos's `reactive_graph` directly, without DOM APIs, a browser window, or an executor. The reactive core itself includes browser console diagnostics on browser WASM targets. The same hooks run on desktop, mobile, browser WASM, WASI, and SSR. The crate requires Rust 1.88 or later.

```rust
use leptos::prelude::*;
use leptoskit_use::{use_toggle, UseToggleReturn};

let UseToggleReturn { value, toggle, .. } = use_toggle(false);
toggle();
assert!(value.get());
```

Use the crate directly or enable `leptoskit`'s `use` feature and import `leptoskit::hooks::*`. Create hooks inside a Leptos `Owner`, just like other owner-scoped signals. Callbacks and captured data must be `Send + Sync`.

## Compatibility

This is an independent implementation, not a drop-in re-export. It mirrors the portable portion of leptos-use and makes environmental inputs explicit. Hooks accept Leptos signals and memos. Generic predicates, string conversion, and signal filters use readable-signal traits for type inference; wrap static inputs to those hooks in `Signal::stored`. Other derived hooks accept `Signal<T>` conversions, including static values. Read derived outputs inside a reactive context to subscribe to updates.

| leptos-use family       | Available here                                                                                        | Differences                                                                                                                                     |
| ----------------------- | ----------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| Toggle                  | `use_toggle`, `UseToggleReturn`, `MaybeRwSignal`                                                      | Initial value or existing `RwSignal`; same value, setter, toggle pattern.                                                                       |
| Cycle list              | `use_cycle_list`                                                                                      | Reactive `Vec`; selection follows index. Empty lists return `None`; set-index wraps. Controls are next, prev, and set-index.                    |
| Sorting                 | `use_sorted`, `use_sorted_by`, `use_sorted_by_key`                                                    | Stable sorted copies of vectors.                                                                                                                |
| Conversion / predicates | `use_to_string`, `is_some`, `is_none`, `is_ok`, `is_err`                                              | Borrow values when possible.                                                                                                                    |
| Logic / math            | `use_not`, `use_and`, `use_or`, `use_abs`, `use_ceil`, `use_floor`, `use_round`, `use_min`, `use_max` | Unary numbers use `f64`; min/max use ordered vectors and return `None` for empty input.                                                         |
| Watch                   | `watch_with_options`, `watch_pausable`, `whenever`, `sync_signal`                                     | Synchronous, executor-free, including SSR; equality suppresses redundant callbacks. Immediate option is a boolean. Stop actions are idempotent. |
| Timers                  | `use_timeout_fn`, `use_interval_fn`, `use_interval`                                                   | Explicit monotonic clock and `Duration`; callbacks take no arguments. Intervals coalesce missed ticks.                                          |
| Filters                 | `use_debounce_fn`, `use_throttle_fn`, `signal_debounced`, `signal_throttled`                          | Trailing debounce; leading-only throttle. Explicit clock, no hidden timers.                                                                     |
| Storage                 | `use_storage`, `Storage`, `MemoryStorage`                                                             | Typed host adapter; fallible operations; persist before updating the signal. Explicit reload for external changes.                              |
| Viewport                | `use_window_size`, `use_breakpoints`                                                                  | Host supplies logical dimensions. Named integer breakpoints support greater-or-equal, smaller, between. Unknown names return `None`.            |

DOM-specific hooks (selectors, CSS, HTML attributes, browser document, mutation/intersection observers) have no renderer-independent meaning and are not exported. Device and network capabilities (clipboard, media, geolocation, permissions, WebSockets, notifications) need real platform adapters and are not yet implemented. There are no native/SSR stubs claiming those capabilities work.

## Driving time

One host clock can drive every timer. Wire its updates to your native event loop, browser frame callback, or server scheduler. A static SSR clock intentionally schedules no background work. The origin is arbitrary; time must be monotonic. Advancing it synchronously runs due callbacks on the calling thread.

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

Timeouts and intervals start on creation. A zero duration fires on the next clock change. Pause/stop cancels the deadline; start/resume schedules from the current time. Watchers unsubscribe on owner cleanup, so disposed timers cannot call back. Handles containing arena signals must not be invoked after their owner is disposed, except watcher stop/pause/resume controls, which tolerate disposal.

Watch callbacks run untracked; reading another signal inside a callback does not subscribe to it. Pausing a watcher continues observing source changes, without invoking the callback. Resuming listens for future changes and does not replay history. Avoid recursively writing a watcher's own source. `sync_signal` handles its feedback by checking equality before writing.

## Storage

Implement `Storage<T>` for your platform's synchronous key/value store; serialization is the adapter's responsibility. `MemoryStorage<T>` is a shared in-memory implementation suitable for headless applications and tests. Allocate a separate store per SSR request to prevent user data crossing requests.

`use_storage` returns an error if the initial read fails. `set_value`, `remove`, and `reload` return adapter errors and preserve the displayed value on failure. Missing values use the initial fallback without writing it. Removing a value resets the signal only after successful removal. Instances do not implicitly synchronize: call `reload` from the host's storage notification handler.

## Verification

`devenv shell coverage:use` enforces 100% line coverage for this crate and emits an LCOV report. CI also runs the suite on Linux, Windows, and macOS, and checks this crate for browser WASM and WASI with default features disabled. Tests cover source reactivity, empty collections, timer cancellation, owner disposal, and storage failures without sleeps.
