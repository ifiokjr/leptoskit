# leptoskit benchmarks

Two kinds of benchmarks live here, with different jobs:

- **The self comparison** runs on every pull request. It builds the fixture app with the base and head builds of the CLI, measures both in headless Chromium, hyperfines the build pipeline for both binaries, and posts a sticky PR comment flagging regressions against `benchmarks/policy.json`. Base and head are measured in the same job, on the same runner, with the same browser — the comparison is always apples to apples.
- **The external comparison** measures leptoskit against the competition — plain React in the DOM, React Native (through react-native-web), Flutter web, and vanilla JS as the floor — plus the build-speed face-off against cargo-leptos. These numbers are **static reference data**: rerun on demand with `devenv shell -- benchmark:docs`, never on every PR. They render into [the benchmarks chapter](../docs/src/benchmarks.md) of the book.

## Layout

```
benchmarks/
  apps/runtime/          one fixture app per framework
    shared/bench.js      the measurement contract every app implements
    leptoskit/           a real leptos client app built by `leptoskit build`
    react/               plain React (vendored ESM bundle, no build step)
    react-native-web/    React Native source rendered by react-native-web
    vanilla-js/          hand-rolled DOM, the floor
    flutter/             flutter web (startup/payload/memory only)
  apps/build/
    cargo-leptos/        the same app sources, built by cargo-leptos
  scripts/               the Deno harness: CDP driver, server, compare, docs
  results/               committed external reference numbers
  policy.json            regression thresholds for the self comparison
```

## What the runtime benchmark measures

The scenario set follows the [js-framework-benchmark] keyed table: create 1,000/10,000 rows, update every 10th, swap two rows, remove one, select one, append 1,000, clear — plus the numbers that shape how an app _feels_:

- **time to first render** — navigation start to the app signalling its interface is in the DOM;
- **click throughput** — how many increment clicks the framework can absorb per second, state update and DOM patch included, measured as a burst of 50,000 clicks on an empty table so handler work dominates the frame that ends the measurement;
- **transfer size** — the compressed bytes the page actually loads;
- **heap after load and after 10,000 rows.**

Every scenario verifies the expected DOM state before its duration counts; a broken app times out instead of posting a fast number. Durations are medians over the configured runs (7 by default), each a fresh page load served with `no-store`, so every load is a cold visit for the app. Noise is handled structurally rather than by hoping: one unrecorded warm-up load absorbs Chromium's own process start-up; when several builds of the same app are measured (base against head), they alternate within every run so machine drift lands on all of them evenly; heap readings force a garbage collection first so they report live memory, not whatever the previous scenario left behind; and the build benchmark samples the tools one run at a time, interleaved, for the same reason.

Flutter web renders to a canvas, so the DOM readers cannot verify its table: it reports startup, transfer, and memory only. Electron is not measured — it is a Chromium-plus-Node shell rather than a browser framework, so its cost shows up in install size and process startup, not in DOM throughput.

## Local commands

| task                 | what it does                                                      |
| -------------------- | ----------------------------------------------------------------- |
| `benchmark:self`     | full base-vs-`origin/main` comparison, prints the PR-style report |
| `benchmark:runtime`  | runtime benchmark for one built dist (`--app`, `--dist`, `--out`) |
| `benchmark:build`    | build-speed hyperfine (`--tool name\|fixture\|command`)           |
| `benchmark:external` | run every present framework, write `benchmarks/results/`          |
| `benchmark:docs`     | `benchmark:external` plus regenerate the docs tables              |
| `benchmark:test`     | unit tests for the comparison and policy logic                    |

The external numbers come from whatever machine ran `benchmark:docs`; the generated tables carry the date, platform, core count, and browser, and a loaded machine inflates everything. Rerun on a quiet one before reading too much into a single column.

Regenerating the vendored React bundles (needs node): `cd
benchmarks/scripts/vendor && npm install && npm run build`.

[js-framework-benchmark]: https://github.com/krausest/js-framework-benchmark
