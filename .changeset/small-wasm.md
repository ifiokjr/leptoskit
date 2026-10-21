---
"release": minor
---

# Ship smaller wasm, and see what you ship

A new `[bundle]` table in `leptoskit.toml` holds everything about what a release build ships. `bundle.optimize` picks how hard the wasm client is optimized: `"fast"`, the default, compiles with your `release` profile, and `"smallest"` adds fat LTO, one codegen unit, `opt-level = "z"`, and `panic = "abort"` through a profile leptoskit defines on cargo's command line. That's 10% to 14% less wasm, for release rebuilds about three times slower. `leptoskit build --optimize` overrides it for one build, and `bundle.cargo_profile` compiles the client with a profile of your own instead. `build.wasm_opt` moves to `bundle.wasm_opt` (a configuration still setting it under `[build]` gets an error naming the new place), `wasm-opt` now runs at `-Oz`, and release wasm drops its name and producers sections.

Release web builds write brotli and gzip copies of every compressible file (`bundle.precompress`, on by default), and dev builds remove them so a stale copy is never served. `leptoskit_axum` now prefers the brotli copy when the browser accepts it.

Every release build prints the client bundle raw, gzipped, and brotli-compressed, totalled as the initial load, and `bundle.budget_kib` fails a release build whose initial load is heavier than the budget.
