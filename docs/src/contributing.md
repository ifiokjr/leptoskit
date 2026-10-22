# Contributing

[`devenv`](https://devenv.sh/) provides the whole development environment: the pinned Rust toolchain, the wasm tools, the browser drivers, and every lint the CI runs.

```bash
devenv shell install:all   # pinned tools and locked dependencies
devenv shell test:all      # nextest plus doctests
devenv shell lint:all      # clippy -D warnings, leptoskit format, dprint, zizmor, monochange, cargo-deny
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

Coverage is measured on four platforms with `-C instrument-coverage`:

- the `coverage` CI job runs the Linux suite once: the unit and integration tests, the GTK widget suite under Xvfb, the doc tests, the portable widget journey through `leptoskit test run`, the CLI's ignored wasm end-to-end tests (`cargo test -p leptoskit_cli --test cli --test render --test wasm_pipeline --test size -- --ignored`), and the islands release build (the CLI those drive is instrumented; the wasm32 builds it starts are not);
- the browser job runs the Chrome suites (`coverage:browser`: leptos_testing_library, the web adapter, and browser storage) with the pinned nightly that wasm-bindgen-test coverage needs;
- the iOS job runs the UIKit verifier (`verify_simulator --coverage`) on a simulator;
- the Android job runs the Android verifier (`verify_device --coverage`) on an emulator.

The `coverage gate` job merges the four reports, so a line is covered when any platform ran it. [`setup/coverage_gate`](https://github.com/ifiokjr/leptoskit/tree/main/setup/coverage_gate) then holds the merged result to two rules, and the job summary shows the per-package table and every uncovered changed line:

- **Workspace floor.** Line coverage of the workspace sources, including the example apps (test code, benchmarks, and the example targets inside crates excluded), may not drop below the value in `setup/coverage_gate/floor.txt`. When a change raises coverage, ratchet the floor up to the merged total: download the run's `coverage-linux`, `coverage-browser`, `coverage-ios`, and `coverage-android` artifacts, run `cargo run -p coverage_gate -- --floor setup/coverage_gate/floor.txt --write-floor` with one `--lcov` per report, and commit the result. (The Linux report names files by absolute path; rewrite its `SF:/home/runner/work/leptoskit/leptoskit/` prefix to your checkout first.)
- **Patch coverage.** At least 90% of the instrumented lines a pull request adds must run under test. A stacked pull request is measured against the layer below it.

`leptoskit_use` is held to 100% separately, natively (`coverage:use`) and in the browser (`coverage:use:browser`).

The Windows and macOS desktop suites, the Firefox runs, and the other iOS and Android steps (the platform probe, the widget journey, the generated app tests, and code push) are not instrumented: their failures fail CI, but their coverage is not counted, so code that only compiles for Windows or macOS shows up in the job summary as changed files without a coverage record.

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
