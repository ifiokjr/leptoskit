# The leptoskit DevTools interface

The page `leptoskit dev`'s DevTools hub serves at `/`, and the browser DevTools panel frames with `?embed=panel`. It is a leptoskit app, written with leptoskit's own components and Tailwind classes, and built for the web.

- `src/model.rs`: what the interface shows, and how the hub's stream changes it.
- `src/view.rs`: the screens.
- `src/store.rs`: the actions the controls run.
- `src/hub.rs`: the hub API, the change stream, and the panel's `postMessage` contract.

Build it, then run any app's `leptoskit dev` with the build:

```sh
devenv shell devtools:build
LEPTOSKIT_DEVTOOLS_UI=$PWD/target/devtools/ui leptoskit dev
```

The tests render every screen on the headless backend and drive its controls:

```sh
cargo test --manifest-path devtools/Cargo.toml
```

Releases build it once, upload `leptoskit-devtools-<version>.tar.gz`, and compile its SHA-256 into the CLI, which downloads it on first use. See [the plan](../docs/agents/devtools-plan.md#distributing-the-interface).
