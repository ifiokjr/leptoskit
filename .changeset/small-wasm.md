---
"release": minor
---

# Ship the smallest wasm, and see what you ship

Release builds compile the wasm client with `leptoskit-wasm`, a size-tuned cargo profile passed on the command line (`release` with `opt-level = "z"`, fat LTO, one codegen unit, `panic = "abort"`), so no manifest needs editing; `build.wasm_profile` names a profile of your own instead. `wasm-opt` now runs at `-Oz`, and release wasm drops its name and producers sections.

Release web builds write brotli and gzip copies of every compressible file (`build.precompress`, on by default), and dev builds remove them so a stale copy is never served. `leptoskit_axum` now prefers the brotli copy when the browser accepts it.

Every release build prints the client bundle raw, gzipped, and brotli-compressed, totalled as the initial load, and `build.budget_kib` fails a release build whose initial load is heavier than the budget.
