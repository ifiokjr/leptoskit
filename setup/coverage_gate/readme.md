# coverage_gate

The CI check that stops workspace test coverage from falling, and makes a pull request's new code arrive with tests.

Coverage here is measured on four platforms: the Linux test suite under `cargo llvm-cov`, the Chrome suites, the UIKit verifier on an iOS simulator, and the Android verifier on an emulator. Each writes an lcov tracefile. This binary merges them, so a line counts as covered when any platform ran it, and holds the result to two rules:

- **The floor.** Workspace line coverage may not fall below the percentage committed in `floor.txt`. Raise it as coverage improves.
- **The patch minimum.** With `--base <rev>`, at least 90% (`--patch-minimum`) of the instrumented lines a pull request adds, found by diffing against the merge base, must be covered. A stacked pull request is measured against the layer below it.

`exclusions.txt` lists path prefixes whose lines count neither way. Everything in it must be code no measuring platform can execute, such as the device step operations that shell out to `simctl`, `devicectl`, and `adb`; anything a CI lane can run belongs in its tests instead.

```text
usage: coverage_gate --lcov <file> [--lcov <file>...] --floor <file> [options]

  --base <rev>           also gate the lines added since the merge base with <rev>
  --exclude-from <file>  path prefixes whose lines count neither way
  --patch-minimum <pct>  the share of added lines that must be covered (default 90)
  --markdown <file>      append the markdown report to <file>, such as $GITHUB_STEP_SUMMARY
  --annotate             print GitHub annotations for uncovered added lines
  --write-floor          set the floor to the measured total instead of checking it
```

CI runs it in the `coverage gate` job of `.github/workflows/ci.yml`, after the four platform jobs upload their reports, with `--markdown "$GITHUB_STEP_SUMMARY" --annotate` so the job summary shows a per-package table and every uncovered changed line. To check locally on Linux:

```sh
devenv shell coverage:all                       # run the workspace under llvm-cov, then the floor check
devenv shell coverage:all --base origin/main    # and gate the lines your branch adds
devenv shell coverage:gate                      # re-check an existing target/coverage/lcov.info
```

A local run on another OS compiles different platform modules, so its total differs from the floor, which the Linux CI job measures. To ratchet the floor after a change raises coverage, run it with `--write-floor` over the merged CI reports and commit the new `floor.txt`; [CONTRIBUTING.md](../../CONTRIBUTING.md#coverage) has the steps. `cargo test -p coverage_gate` runs its own tests.
