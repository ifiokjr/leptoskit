# hooks_guest

A compiled Wasm guest (package `hooks_guest_probe`) that uses `leptoskit::hooks`, kept to prove the native interpreter can run an app built on reactive state.

The hooks bring `reactive_graph` into the guest, and its `wasm32-unknown-unknown` build links wasm-bindgen runtime imports that no interpreter provides. If a dependency bump adds an import the interpreter does not tolerate, an app using hooks would fail to load on a device. This guest makes that failure show up in CI instead. It mounts a switch (`use_toggle`, `use_not`) whose label comes from a `Memo` and whose change count comes from `watch_with_options`, and exports the same entry points a shipped bundle does (`leptoskit_main`, `leptoskit_event`, `leptoskit_alloc`, `leptoskit_abi_version`).

It is not an app and has no host target: the crate is empty outside `wasm32`. The ignored test `compiled_hooks_guest_instantiates_and_renders_reactive_state` in `crates/leptoskit_wasm/tests/hooks_guest.rs` loads the built `.wasm`, checks the tree reads `Off` and `Changes: 0`, presses the button twice, and checks `On`/`Changes: 1`, then `Off`/`Changes: 2`.

```sh
devenv shell test:plugins:guest
```

That task builds this crate for `wasm32-unknown-unknown` (a debug build, so every import the dependencies link is kept), then runs the test with `LEPTOSKIT_HOOKS_GUEST` pointing at the result. CI runs the same task in its `test compiled plugin guest` step.

With its `devtools` feature it also exposes the change count as `changes` and tracks a two-screen router, and the ignored test in `crates/leptoskit_bridge_ffi/tests/devtools_guest.rs` runs it in a real app reporting to a fake hub: the tree, a commit, the guest's state and route, and `SetState` and `Navigate` from the hub. `devenv shell test:plugins:guest` builds it that way too.
