# Contributing to leptoskit

This file is for people working on leptoskit itself. The [public docs](https://ifiokjr.github.io/leptoskit/) are for people building apps with it, so repository workflows, CI details, and maintenance steps live here and in [`docs/agents/`](docs/agents/readme.md) instead. Coding agents should also read [`AGENTS.md`](AGENTS.md).

## Development environment

[`devenv`](https://devenv.sh/) provides the whole development environment: the pinned Rust toolchain, the wasm tools, the browser drivers, and every lint the CI runs.

```bash
devenv shell install:all   # pinned tools and locked dependencies
devenv shell test:all      # nextest plus doctests
devenv shell lint:all      # clippy -D warnings, leptoskit format, dprint, monochange, cargo-deny
devenv shell fix:all       # autofix everything fixable
```

| Script                 | What it does                                                       |
| ---------------------- | ------------------------------------------------------------------ |
| `test:browser`         | Runs the `leptoskit_testing_library` suite in Chrome.              |
| `coverage:all`         | Workspace coverage, held to the coverage gate (below).             |
| `docs:build`           | Builds the docs site into `docs/dist`.                             |
| `docs:serve`           | Serves the docs site on port 4400, rebuilt and reloaded on edits.  |
| `schema:update`        | Regenerates the public configuration schemas from the Rust types.  |
| `schema:check`         | Fails when the committed schemas drift from the Rust types.        |
| `ios_sim_run`          | Runs the platform probe on the iOS simulator.                      |
| `android_emulator_run` | Runs the platform probe on an Android device or emulator.          |
| `test:harmonyos`       | Checks HarmonyOS render mutations and widget state without an SDK. |
| `bench:build`          | Benchmarks cached and incremental web builds with `hyperfine`.     |
| `benchmark:docs`       | Reruns the comparison benchmarks and regenerates the docs tables.  |
| `security:zizmor`      | Audits the GitHub Actions workflows and composite actions offline. |

Editor settings live in `setup/editors`. Run `setup:vscode` or `setup:helix` to copy them into place.

Depend on a local checkout from a generated app with `--sdk-path`: `leptoskit new my_app --sdk-path /path/to/leptoskit`. The plugin generator takes `--bridge-path /path/to/crates/leptoskit_bridge` for the same purpose.

## Coverage

Coverage is measured on four platforms with `-C instrument-coverage`:

- the `coverage` CI matrix runs the Linux suite once across two jobs: `workspace` runs the unit and integration tests, the GTK widget suite under Xvfb, the doc tests, and the portable widget journey through `leptoskit test run`; `applications` runs the CLI's ignored wasm end-to-end tests, generated Linux and terminal app builds, opencode's browser and terminal builds, the islands release build, and the file-routes Linux build (the CLI those drive is instrumented; the wasm32 builds it starts are not);
- the browser job runs the Chrome suites (`coverage:browser`: leptoskit_testing_library, the web adapter, and browser storage) with the pinned nightly that wasm-bindgen-test coverage needs;
- the iOS widget job runs the UIKit verifier (`verify_simulator --coverage`) on a simulator; separate jobs run generated application journeys and the code push milestone, and the coverage gate waits for all three;
- the Android job runs the Android verifier (`verify_device --coverage`) on an emulator.

The `coverage gate` job merges both Linux reports with the browser, iOS, and Android reports, so a line is covered when any platform ran it. [`setup/coverage_gate`](setup/coverage_gate) then holds the merged result to two rules, and the job summary shows the per-package table and every uncovered changed line:

- **Workspace floor.** Line coverage of the workspace sources, including the example apps (test code, benchmarks, and the example targets inside crates excluded), may not drop below the value in `setup/coverage_gate/floor.txt`. When a change raises coverage, ratchet the floor up to the merged total: download the run's `coverage-linux-workspace`, `coverage-linux-applications`, `coverage-browser`, `coverage-ios`, and `coverage-android` artifacts, run `cargo run -p coverage_gate -- --floor setup/coverage_gate/floor.txt --write-floor` with one `--lcov` per report, and commit the result. (The Linux reports name files by absolute path; rewrite their `SF:/home/runner/work/leptoskit/leptoskit/` prefix to your checkout first.)
- **Patch coverage.** At least 90% of the instrumented lines a pull request adds must run under test. A stacked pull request is measured against the layer below it.

`leptoskit_use` is held to 100% separately, natively (`coverage:use`) and in the browser (`coverage:use:browser`).

The Windows and macOS desktop suites, the Firefox runs, and the other iOS and Android steps (the platform probe, the widget journey, the generated app tests, and code push) are not instrumented: their failures fail CI, but their coverage is not counted, so code that only compiles for Windows or macOS shows up in the job summary as changed files without a coverage record.

## Snapshot tests

Integration tests that generate output, such as scaffolded projects, help text, error messages, and code push manifests, capture it with [insta](https://insta.rs). A change to anything the CLI generates shows up as a diff in a committed `.snap` file instead of passing silently. The projects those tests run against live in `crates/leptoskit_cli/tests/fixtures`, and the snapshots in `crates/leptoskit_cli/tests/snapshots`.

Plain `cargo test` fails on a missing or changed snapshot, so CI cannot pass with an unreviewed output change. When a change is intentional, record and accept it:

```bash
cargo insta test --accept   # or: INSTA_UPDATE=always cargo test --package leptoskit_cli
cargo insta review          # walk the diffs and accept each one
```

Snapshots are reviewed like code: a diff in a `.snap` file is the reviewable record of what the CLI generates.

## The docs site

The public site lives in [`docs/`](docs): the book in `docs/src`, and the binary that builds it in `docs/site` (built on [`leptoskit_docs`](crates/leptoskit_docs)).

```bash
devenv shell docs:serve   # port 4400, rebuilt and reloaded on every edit
devenv shell docs:build   # docs/dist, as it is published
```

The `docs` workflow builds the site on every pull request, so a broken link fails the check, and publishes it to GitHub Pages from `main`.

The site is for people using leptoskit. Keep repository workflows, CI job descriptions, `devenv` commands, links to test files, and design investigations out of it; they belong in this file or in [`docs/agents/`](docs/agents/readme.md). The [docs authoring rules](docs/agents/docs-site.md) cover the strict build: closed tags, `.md` links, local addresses as inline code, and dprint's indentation trap.

## Configuration schema

The JSON Schema for `leptoskit.toml` is generated from the Rust configuration types in `leptoskit_config`. Generate and check the assets from the repository root:

```sh
devenv shell schema:update
devenv shell schema:check
devenv shell docs:build
```

Do not edit generated JSON by hand. `schema:check` runs in local lint and pre-push checks and in CI. Tests validate real project configurations against the schema and exercise historical migrations. The docs workflow verifies that the built site contains the exact committed schema bytes.

While the schema is unstable, update the v0 contract and regenerate the assets when configuration changes. For the first stable version 1 schema and subsequent breaking configuration changes:

1. Increment `SCHEMA_VERSION` in `leptoskit_config`. The version constraint and pinned editor URL derive from it.
2. Add the next explicit step to `migration::migrate`. Keep every older step, and reject missing steps and future versions.
3. Add a historical fixture and verify that migration preserves its meaning and comments, rejects conflicts, and is idempotent.
4. Run `schema:update`. It updates both latest aliases and writes the current major's versioned asset. It never removes or rewrites older majors.
5. Preserve published older contracts and add the new major URL to the [schema page](docs/src/reference/schemas.md). Update schema tests to include the new asset.

## Benchmarks

The [benchmarks page](docs/src/benchmarks.md) publishes two kinds of numbers:

- **Against itself, on every pull request.** The `benchmarks` workflow rebuilds the fixture app with the base and head builds of the CLI and measures both in the same job, on the same runner, in the same headless Chromium. Regressions against [`benchmarks/policy.json`](benchmarks/policy.json) are flagged in a sticky PR comment, and fail-level regressions fail the job. The escape hatch is `LEPTOSKIT_ALLOW_BENCH_REGRESSIONS=true`.
- **Against the competition, on demand.** The tables on the page are static reference data, regenerated with `devenv shell -- benchmark:docs`, never on every PR. The methodology lives in [`benchmarks/README.md`](benchmarks/README.md).

The native table has a column for every `benchmarks/results/native-<platform>.json` present. `benchmark:native` measures the host, and `benchmark:native:ios` runs on the booted iOS simulator. The `device comparison` workflow (`workflow_dispatch`) runs both the simulator and an Android emulator and uploads the JSON to commit. The pull-request comment also carries this section, measured on the CI host for base and head. `benchmark:size` regenerates the app-size table. `cargo bench -p leptoskit_fmt` runs the formatter benchmarks quoted in the formatting guide.

## Platform validation

[`docs/agents/platform-validation.md`](docs/agents/platform-validation.md) describes the platform probe, the native fixtures, and what each CI job runs on each platform. [`docs/agents/harmonyos.md`](docs/agents/harmonyos.md) covers the HarmonyOS SDK checks and CI.

## Releases

Releases go through [monochange](https://crates.io/crates/monochange) changesets:

1. A feature branch carries a changeset: `monochange run change --package leptoskit --bump patch --reason "..."`.
2. The `release` workflow keeps one release pull request open that summarises the pending changesets. Merging it cuts the release.
3. The merge pushes the tag, drafts the GitHub release, and dispatches the `publish` workflow. That workflow publishes every crate to crates.io in dependency order.

Every published crate releases together as one version group under a single tag.
