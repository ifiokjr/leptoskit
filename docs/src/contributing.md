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
| `coverage:all`         | Workspace coverage, held to the coverage gate (below).         |
| `docs:build`           | Builds this site into `docs/book`.                             |
| `docs:serve`           | Serves this site with live reload.                             |
| `ios_sim_run`          | Runs the platform probe on the iOS simulator.                  |
| `android_emulator_run` | Runs the platform probe on an Android device or emulator.      |
| `bench:build`          | Benchmarks cached and incremental web builds with `hyperfine`. |

Editor settings live in `setup/editors`. Run `setup:vscode` or `setup:helix` to copy them into place.

## Coverage

The `coverage` CI job runs the Linux test suite once with `-C instrument-coverage`: the unit and integration tests, the GTK widget suite under Xvfb, the doc tests, and the portable widget journey through `leptoskit test run`. [`setup/coverage_gate`](https://github.com/ifiokjr/leptoskit/tree/main/setup/coverage_gate) then holds the result to two rules, and the job summary shows the per-package table and every uncovered changed line:

- **Workspace floor.** Line coverage of the workspace sources, including the example apps (test code, benchmarks, and the example targets inside crates excluded), may not drop below the value in `setup/coverage_gate/floor.txt`. When a change raises coverage, ratchet the floor up to the new total with `coverage:gate --write-floor` against the CI job's `lcov.info` artifact, and commit it.
- **Patch coverage.** At least 90% of the instrumented lines a pull request adds must run under test. A stacked pull request is measured against the layer below it.

`leptoskit_use` is held to 100% separately, natively (`coverage:use`) and in the browser (`coverage:use:browser`).

The iOS simulator, Android emulator, Windows, macOS, and browser suites are not instrumented: their failures fail CI, but their coverage is not counted, so code that only compiles for those platforms shows up in the job summary as changed files without a coverage record.

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
