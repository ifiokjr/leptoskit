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

## Snapshot tests

Integration tests that generate output — scaffolded projects, help text, error messages, code push manifests — capture it with [insta](https://insta.rs) so a change to anything the CLI generates shows up as a diff in a committed `.snap` file instead of passing silently. The projects those tests run against live in `crates/leptoskit_cli/tests/fixtures`, and the snapshots in `crates/leptoskit_cli/tests/snapshots`.

Plain `cargo test` fails on a missing or changed snapshot, so CI cannot pass with an unreviewed output change. When a change is intentional, record and accept it:

```bash
cargo insta test --accept   # or: INSTA_UPDATE=always cargo test --package leptoskit_cli
cargo insta review          # walk the diffs and accept each one
```

Snapshots are reviewed like code: a diff in a `.snap` file is the reviewable record of what the CLI generates.

## Releases

Releases go through [monochange](https://crates.io/crates/monochange) changesets:

1. A feature branch carries a changeset: `monochange run change --package leptoskit --bump patch --reason "..."`.
2. The `release` workflow keeps one release pull request open that summarises the pending changesets. Merging it cuts the release.
3. The merge pushes the tag, drafts the GitHub release, and dispatches the `publish` workflow. That workflow publishes every crate to crates.io in dependency order.

All eleven crates release together as one version group under a single tag.
