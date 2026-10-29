# leptoskit_bench_guest

The workload the native benchmark drives through the rendering ABI: a 1,000-row list, a single-label update per event, and a full re-render.

It is a plain Rust `cdylib` with no framework on top, written against the raw `leptoskit` imports (`node_create`, `node_set_props`, `node_insert`, `commit`), so the Wasm layer is measured running real Rust codegen the way a shipped bundle does rather than a hand-written module. Node IDs are fixed: the root is `1`, row `i` is `2 + i`, and its label is `LABEL_BASE + i`. A press on the root updates one label; a press on the first row rewrites every label. The JavaScript guest in `leptoskit_bench` (`src/js_layer.rs`) mirrors this file call for call, so the Wasm and QuickJS numbers compare the same work.

The crate compiles only for `wasm32`. You do not build it by hand: `devenv shell benchmark:native` builds it for `wasm32-unknown-unknown` and passes the path to `leptoskit_bench` as `LEPTOSKIT_BENCH_GUEST_WASM`. See [`../readme.md`](../readme.md).
