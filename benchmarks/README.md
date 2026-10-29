# leptoskit benchmarks

Two kinds of benchmarks live here, with different jobs:

- **The self comparison** runs on every pull request. It builds the fixture app with the base and head builds of the CLI, measures both in headless Chromium, hyperfines the build pipeline for both binaries, and posts a sticky PR comment flagging regressions against `benchmarks/policy.json`. Base and head are measured in the same job, on the same runner, with the same browser — the comparison is always apples to apples.
- **The arena** measures leptoskit against React Native, Lynx, and Flutter on device, with one self-driving app per framework, on iOS, Android, macOS, Windows, Linux, and the web: launch, rendering, native-call round trips, presented frames from screen recordings, and the size of the same app at three sizes. Its design and app contract are in [`arena/README.md`](arena/README.md). Run it on demand with `devenv shell -- benchmark:arena`; `benchmark:arena:update` first moves every framework to its latest stable release.
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
  arena/                 leptoskit against React Native, Lynx, and Flutter on device
    apps/<framework>/    one scenario app per framework, its arena.json, and its tier shell
    spec/app.json        the size-tier specification every framework generates from
    versions.json        the framework releases measured
  native/                leptoskit_bench: the view, wasm, tap, and JavaScript layers
    guest/               the Rust wasm guest driving the rendering ABI
    counter/             the starter counter screen, for the tap scenarios
  scripts/               the Deno harness: CDP driver, server, compare, docs
  results/               committed reference numbers (external + native-<platform> + sizes)
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

## What the size benchmark measures

`benchmark:size` answers "what does a minimal app weigh?" on every platform leptoskit publishes to. It scaffolds a fresh app per platform — the web template through `leptoskit new`, the native template through `leptoskit new --platform native` — builds it in release with the CLI under test, and walks the artifact that platform actually ships:

- **web** — the client bundle under `dist/pkg/`, raw and brotli-compressed as served, totalling the initial load;
- **macOS / iOS** — the `.app` bundle: the linked binary, the `app.wasm` guest, and the `app.lkb` code push baseline;
- **Android** — the APK as downloaded, plus the uncompressed `lib*.so` and the assets it carries, read from its central directory.

Apple builds need Xcode; Android builds need `ANDROID_HOME` with an NDK. A platform whose toolchain is missing is skipped with a note recorded in the results. `leptoskit native build --release` prints the same report for any project, so a size regression shows up in the build that caused it.

## The view layer, the wasm runtime, and a JavaScript baseline

On device, a leptoskit interface is a wasm module interpreted by wasmi driving `leptoskit_native` over `leptoskit_yoga`. `benchmarks/native` is one Rust binary, `leptoskit_bench`, that measures those layers with the same 1,000-row workload as the browser suite and runs unchanged on the host, in the iOS simulator, and on an Android emulator (through `leptoskit test run`):

- **native / yoga** — build, commit, relayout, diff a partial update and a removal, Tailwind resolution, event dispatch, hit testing; the layout engine standalone (flat, grid, incremental).
- **wasm** — a Rust guest compiled to wasm (`native/guest`) building the list, handling presses, and re-rendering through the real ABI in wasmi.
- **tap** — the counter screen `leptoskit new` generates (`native/counter`), built with the same components and guest runtime as a new app: a tap presses its button, the guest re-renders, the host commits, and the changes are encoded the way the shell receives them. Besides the comparison against the base branch, a tap has a hard budget of one 60 Hz frame; a slower tap fails the run.
- **js** — the same guest written in JavaScript, call for call, in QuickJS: an interpreter-class engine like Hermes, so "wasm layer versus JavaScript layer" is answered on identical hardware with an identical host. It is a stand-in for that engine class, not a Hermes measurement.

Every scenario verifies the surface afterwards. The guests are embedded at build time (`LEPTOSKIT_BENCH_GUEST_WASM` and `LEPTOSKIT_BENCH_COUNTER_WASM`, set by the harness) so the binary carries its workload onto a device. Device builds generate the QuickJS bindings with bindgen and need a host libclang (Xcode's on macOS, the system LLVM's on Linux). Simulators and emulators run on the host CPU; physical devices differ.

The host run is part of every pull request's comparison; `benchmark:native`, `benchmark:native:ios`, and `benchmark:native:android` refresh the committed `results/native-<platform>.json`, and the `device comparison` workflow runs the simulator and emulator on demand.

## Local commands

| task                       | what it does                                                                       |
| -------------------------- | ---------------------------------------------------------------------------------- |
| `benchmark:self`           | full base-vs-`origin/main` comparison, prints the PR-style report                  |
| `benchmark:runtime`        | runtime benchmark for one built dist (`--app`, `--dist`, `--out`)                  |
| `benchmark:build`          | build-speed hyperfine (`--tool name\|fixture\|command`)                            |
| `benchmark:external`       | run every present framework, write `benchmarks/results/`                           |
| `benchmark:size`           | minimal app size on web, macOS, iOS, Android → `results/sizes.json`                |
| `benchmark:native`         | view, wasm, and JavaScript layers on this host → `results/native-host.json`        |
| `benchmark:native:ios`     | the same on the booted iOS simulator → `results/native-ios.json`                   |
| `benchmark:native:android` | the same on an Android device/emulator → `results/native-android.json`             |
| `benchmark:docs`           | `benchmark:native` + `benchmark:external`, then regenerate the docs tables         |
| `benchmark:arena`          | the arena on every reachable target → `results/arena-<platform>.json`, sizes, docs |
| `benchmark:arena:update`   | move the arena to the latest framework releases, then run it                       |
| `benchmark:arena:docs`     | regenerate the arena tables from the committed results                             |
| `benchmark:test`           | unit tests for the comparison, policy, and arena logic                             |

The external numbers come from whatever machine ran `benchmark:docs`; the generated tables carry the date, platform, core count, and browser, and a loaded machine inflates everything. Rerun on a quiet one before reading too much into a single column.

Regenerating the vendored React bundles (needs node): `cd
benchmarks/scripts/vendor && npm install && npm run build`.

[js-framework-benchmark]: https://github.com/krausest/js-framework-benchmark
