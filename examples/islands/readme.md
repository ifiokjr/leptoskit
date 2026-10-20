# islands

A leptos islands app built by leptoskit. Every page renders on the server, and the wasm client carries only the `#[island]` components: the prose around them never ships a byte of wasm.

```bash
cd examples/islands
leptoskit dev      # build, run the server, rebuild on change
leptoskit build --release
leptoskit serve    # run the release server over dist/
```

`leptoskit.toml` sets `render = "islands"`, so `leptoskit build` compiles the server with the `ssr` feature, the wasm client with `hydrate`, and both with `leptos/islands`. One crate is both sides: the library is the client, and `src/main.rs` is the server.

The colour mixer on `/lab` is `#[island(lazy)]`, and `split = true` moves it into a wasm chunk of its own: the notes page never downloads it. `budget_kib = 80` fails a release build whose initial load outgrows 80 KiB brotli-compressed, and CI builds the example for release on every pull request.

The [small bundles guide](https://ifiokjr.github.io/leptoskit/guide/07-small-bundles.html) walks through it.
