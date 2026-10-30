# leptoskit_bench_counter

The starter app's counter screen as a Wasm guest, so the tap benchmark measures what a tap costs in a new app.

It builds the screen `leptoskit new` generates (a title, a `Count: N` label, an `Add one` button, the same components and Tailwind classes) with the same guest runtime, and exports the entry points the interpreter calls. The [`tap`](../src/tap.rs) scenario in `leptoskit_bench` mounts it, presses the button, and times the commit and the encoding of the changes for the shell against a one-frame budget.

The crate compiles only for `wasm32`; on the host it is empty. You do not build it by hand: `devenv shell benchmark:native` builds it for `wasm32-unknown-unknown` and hands the path to `leptoskit_bench` as `LEPTOSKIT_BENCH_COUNTER_WASM`. See [`../readme.md`](../readme.md).
