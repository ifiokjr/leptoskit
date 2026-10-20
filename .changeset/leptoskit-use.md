---
"release": minor
---

# Add cross-platform reactive utilities

Add `leptoskit_use` and the optional `leptoskit::hooks` re-export. State, derived values, watchers, timer filters, typed storage, and viewport breakpoints use Leptos signals without requiring a DOM or executor. Host clocks, viewport signals, and storage adapters make environmental dependencies explicit. CI enforces 100% line coverage for the new crate and checks browser WASM and WASI builds.
