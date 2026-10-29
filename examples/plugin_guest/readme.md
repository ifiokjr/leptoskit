# plugin_guest

A compiled Wasm guest (package `plugin_guest_probe`) that exercises the plugin bridge from the guest side the way a real app would: real async calls, event wake-ups, and cancellation by dropping a future.

The bridge's host-side behavior is easy to test in Rust. What needs a compiled guest is the other half: `leptoskit_bridge::guest::Client` and `Events` running in the interpreter, polled by the app's own executor and woken by the host's `leptoskit_plugins_ready`. This probe registers a tiny single-task executor and, against an `echo` plugin the test supplies, checks that:

- calling a method the host did not grant fails with `PermissionDenied`;
- dropping an unpolled call, and a running call, cancels the native request;
- replies come back correlated to their own call (an echo and a cancelled call in flight together);
- only one `Events` receiver can exist (`Busy` otherwise), a cancelled wait does not lose the next event, and a dropped receiver can be taken again.

The crate is empty outside `wasm32`. The ignored test `compiled_rust_guest_handles_async_calls_events_and_cancellation` in `crates/leptoskit_wasm/tests/plugins.rs` supplies the host half and loads the built `.wasm`:

```sh
devenv shell test:plugins:guest
```

The task builds this crate in release mode for `wasm32-unknown-unknown`, then runs the test with `LEPTOSKIT_PLUGIN_GUEST` set to the output. CI runs it in the `test compiled plugin guest` step.
