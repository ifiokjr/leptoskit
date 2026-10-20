---
"release": minor
---

# Harden code push and layout; add the logo and docs

Code push now rejects bundle paths that escape the staging directory, bounds decompression and delta output before allocating, refuses to overwrite a published release, writes releases atomically, offers deltas only against the exact release they were computed from, and falls back to the full download when a delta fails.

The layout engine caches measurements per node, so layout cost no longer grows exponentially with nesting depth. Flex free space, percentage bases, reversed axes, single-line `align-content`, grid placement of partly-auto items, grid row sizing, `minmax` growth, and `display: contents` in grid now follow CSS. Tree cycles and huge grid line numbers are rejected.

The native renderer inserts a replaced element once, relayouts when text changes, and forgets deleted nodes. The wasm host clamps guest indices, bounds-checks guest pointers, rejects tree cycles, derives the root at commit, and reports `OutOfFuel`.

The CLI always lets cargo decide what to recompile and caches only wasm-bindgen and `wasm-opt`, honours `build.client_package`, features, plugin env, and rustflags, restores missing outputs, and scaffolds projects that build. `doctor` exits non-zero when a required tool is missing. `leptoskit_axum` sends `Vary: Accept-Encoding`, limits the SPA fallback to page routes, and errors on an invalid bind address. `leptoskit_fns` retries work in the browser. `leptoskit_config` validates package ids and code push settings.

The project has a logo, a documentation site, and a brand page.
