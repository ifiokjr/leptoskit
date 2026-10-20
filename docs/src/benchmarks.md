# Benchmarks

leptoskit is benchmarked two ways, and the numbers mean different things:

- **Against itself, on every pull request.** The `benchmarks` workflow rebuilds the fixture app with the base and head builds of the CLI and measures both in the same job, on the same runner, in the same headless Chromium. Regressions against [`benchmarks/policy.json`][policy] are flagged in a sticky PR comment; fail-level regressions fail the job (escape hatch: `LEPTOSKIT_ALLOW_BENCH_REGRESSIONS=true`).
- **Against the competition, on demand.** The tables below are static reference data regenerated with `devenv shell -- benchmark:docs` — never on every PR. They compare leptoskit with plain React in the DOM, React Native rendered by react-native-web, Flutter web, and vanilla JS as the floor, plus the build-speed face-off against cargo-leptos.

The methodology lives in [`benchmarks/README.md`][readme]: the scenario set follows the js-framework-benchmark keyed table, every scenario verifies the expected DOM state before its duration counts, and the user-facing metrics — time to first render, click throughput, transfer size, heap — are first-class alongside it.

## Runtime in the browser

All fixtures implement the same keyed-table app with the same labels. Durations are medians over 7 fresh-page runs in headless Chromium; transfer sizes are gzip-compressed, as served. Flutter web renders to a canvas, so its table scenarios cannot be DOM-verified — it reports startup, transfer, and memory only.

<!-- BEGIN GENERATED: benchmarks-runtime -->

|              app | first render | transfer | create 1k | create 10k | update 10th |    swap |  remove |  select | append 1k |   clear |  clicks/s |     heap |
| ---------------: | -----------: | -------: | --------: | ---------: | ----------: | ------: | ------: | ------: | --------: | ------: | --------: | -------: |
|            react |      24.4 ms |  63.3 KB |   16.6 ms |     258 ms |      7.4 ms | 24.0 ms | 18.2 ms | 17.5 ms |   27.3 ms | 14.9 ms |  229779/s | 11.74 MB |
| react-native-web |      41.5 ms | 144.4 KB |   35.1 ms |     425 ms |     17.1 ms | 28.0 ms | 16.1 ms | 17.4 ms |   42.3 ms | 10.7 ms |  239693/s | 79.23 MB |
|       vanilla-js |      20.0 ms |   3.5 KB |   13.2 ms |     118 ms |      6.1 ms | 16.9 ms | 17.5 ms | 16.4 ms |   27.6 ms | 16.7 ms |  816993/s |  2.73 MB |
|        leptoskit |      30.0 ms |  64.2 KB |   18.4 ms |     174 ms |      5.7 ms | 18.0 ms | 18.2 ms | 16.9 ms |   27.0 ms | 16.9 ms | 1184834/s |  5.29 MB |
|          flutter |       164 ms | 646.7 KB |         — |          — |           — |       — |       — |       — |         — |       — |         — |        — |

_Measured 2026-10-20 on darwin aarch64, 12 cores, headless Chrome/155; 7 runs per app, medians._

<!-- END GENERATED: benchmarks-runtime -->

## Build speed

The same app sources — byte-identical, enforced at benchmark time — built through each toolchain on the same machine. **Warm** is a no-change rebuild after a priming build; **incremental** bumps the client source first (the loop developers live in); **cold** removes the target directory and output entirely. The leptoskit pipeline runs wasm-bindgen in-process and skips it entirely when the compiled wasm is unchanged, which is where the warm and incremental gaps come from.

<!-- BEGIN GENERATED: benchmarks-build -->

| tool         | warm rebuild | incremental rebuild | cold build |
| ------------ | -----------: | ------------------: | ---------: |
| leptoskit    |       255 ms |             1072 ms |   31131 ms |
| cargo-leptos |       584 ms |             1192 ms |   38238 ms |

_Measured 2026-10-20 on darwin aarch64, 12 cores; hyperfine wall-clock medians, tools interleaved per round._

<!-- END GENERATED: benchmarks-build -->

[policy]: https://github.com/ifiokjr/leptoskit/blob/main/benchmarks/policy.json
[readme]: https://github.com/ifiokjr/leptoskit/blob/main/benchmarks/README.md
