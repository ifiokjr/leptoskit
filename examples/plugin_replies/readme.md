# plugin_replies

A compiled Wasm guest (package `plugin_replies_probe`) that proves a guest rejects malformed answers from native plugins instead of trusting them.

A plugin reply is just bytes, and a buggy or hostile host can send bytes of the wrong shape. The typed clients in `leptoskit_plugin_filesystem`, `leptoskit_plugin_haptics`, and `leptoskit_plugin_sqlite` are supposed to check them: an acknowledgement for a write must be empty, and a SQLite transaction must return one result per statement. This probe calls those clients through the real bridge twice. First every reply is malformed and each call must resolve to `Error::InvalidArgument`; then every reply is valid and each call must succeed (a two-statement transaction returns two results).

The crate is empty outside `wasm32`. The ignored test `compiled_rust_guest_validates_plugin_acknowledgements_and_batch_cardinality` in `crates/leptoskit_wasm/tests/plugin_replies.rs` registers the three plugins, answers each request from the host side, and loads the built `.wasm`:

```sh
devenv shell test:plugins:guest
```

The task builds this crate in release mode for `wasm32-unknown-unknown`, then runs the test with `LEPTOSKIT_PLUGIN_REPLY_GUEST` set to the output. CI runs it in the `test compiled plugin guest` step.
