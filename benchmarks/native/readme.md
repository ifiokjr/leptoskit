# leptoskit_bench

The native-layer benchmark: how much time leptoskit spends outside the browser. One binary, `leptoskit_bench`, measures the layers a native app stacks, and runs unchanged on the host, in the iOS simulator, and on an Android emulator.

The browser benchmarks in the parent directory cover the web. A native app is different: the interface runs as a Wasm guest in an interpreter, its output is committed to a widget tree, and layout and diffing happen in Rust, so a regression in any layer shows up as a slow tap. This binary times each layer on its own:

| Layer                          | `--only`         | What it measures                                                                                                                                                                                                                                                                                       |
| ------------------------------ | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| View (`src/view.rs`)           | `native`, `yoga` | `leptoskit_native` and `leptoskit_yoga` on the headless backend with the browser benchmark's 1,000-row list: build, commit, relayout, update, remove, and the dispatch and hit-test paths; flat, grid, and incremental layout.                                                                         |
| Wasm (`src/wasm_layer.rs`)     | `wasm`           | `leptoskit_wasm` running the Rust guest in [`guest/`](guest) through the real rendering ABI: instantiate, start (1,000 rows), 100 press round trips, event throughput, and a full re-render. It verifies the surface after each scenario, so a guest that stopped rendering cannot post a fast number. |
| Tap (`src/tap.rs`)             | `tap`            | Pressing the button of the starter counter ([`counter/`](counter)) until the new count is committed and encoded for the shell. A tap has a hard budget of one 60 Hz frame; the run fails if it takes longer.                                                                                           |
| JavaScript (`src/js_layer.rs`) | `js`             | The same workload as JavaScript in QuickJS, an interpreter-class engine, to compare the Wasm layer with on the same device. Behind the default `js` feature; turn it off where there is no C toolchain.                                                                                                |

## Run it

The harness builds the guests and embeds them (`build.rs` reads `LEPTOSKIT_BENCH_GUEST_WASM` and `LEPTOSKIT_BENCH_COUNTER_WASM`, so the binary carries its workload onto a device), then runs the binary and writes one `BenchmarkRun` JSON:

```sh
devenv shell benchmark:native               # this host, into benchmarks/results/native-host.json
devenv shell benchmark:native:ios           # the booted iOS simulator
devenv shell benchmark:native:android emulator-5554 --android-arch x86_64
```

To run the binary directly, pass the flags below. Without the two environment variables the Wasm and tap scenarios skip themselves and say so in the report.

```sh
cargo run --release -p leptoskit_bench -- --runs 15 --warmup 2 --only native,wasm --out run.json
```

`--out` writes the JSON; without it, device runs print the JSON between `LEPTOSKIT_BENCH_JSON_BEGIN` and `LEPTOSKIT_BENCH_JSON_END` lines for the harness to harvest. `--subject` names the run.

CI runs the same harness two ways. On every pull request the self comparison builds the base and head binaries first and measures them in alternating rounds, so a shift in the runner's speed lands on both sides. A manual `device comparison` dispatch runs it on the iOS simulator and an Android emulator; commit the output to `benchmarks/results/` to refresh the book's table. Simulators run on the host CPU, so physical devices will differ. See [`benchmarks/README.md`](../README.md) for the rest of the suite.
