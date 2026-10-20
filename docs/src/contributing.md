# Contributing

[`devenv`](https://devenv.sh/) provides the whole development environment: the pinned Rust toolchain, the wasm tools, the browser drivers, and every lint the CI runs.

```bash
devenv shell install:all   # pinned tools and locked dependencies
devenv shell test:all      # nextest plus doctests
devenv shell lint:all      # clippy -D warnings, dprint, zizmor, monochange, cargo-deny
devenv shell fix:all       # autofix everything fixable
```

| Script                 | What it does                                                   |
| ---------------------- | -------------------------------------------------------------- |
| `test:browser`         | Runs the `leptos_testing_library` suite in Chrome.             |
| `coverage:all`         | Workspace coverage, with a 70% line-coverage floor.            |
| `docs:build`           | Builds this site into `docs/book`.                             |
| `docs:serve`           | Serves this site with live reload.                             |
| `ios_sim_run`          | Runs the platform probe on the iOS simulator.                  |
| `android_emulator_run` | Runs the platform probe on an Android device or emulator.      |
| `bench:build`          | Benchmarks cached and incremental web builds with `hyperfine`. |

Editor settings live in `setup/editors`. Run `setup:vscode` or `setup:helix` to copy them into place.

## Releases

Releases go through [monochange](https://crates.io/crates/monochange) changesets:

1. A feature branch carries a changeset: `monochange run change --package leptoskit --bump patch --reason "..."`.
2. The `release` workflow keeps one release pull request open that summarises the pending changesets. Merging it cuts the release.
3. The merge pushes the tag, drafts the GitHub release, and dispatches the `publish` workflow. That workflow publishes every crate to crates.io in dependency order.

All eleven crates release together as one version group under a single tag.
