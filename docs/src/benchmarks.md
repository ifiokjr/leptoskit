# Benchmarks

leptoskit is benchmarked against itself on every pull request, so a change that slows it down is caught before it merges. The tables on this page are the other half: a comparison, run on demand, of what leptoskit costs next to the alternatives. They compare leptoskit with plain React in the DOM, React Native rendered by react-native-web, Flutter web, and vanilla JS as the floor, plus the build-speed face-off against cargo-leptos.

The methodology lives in [`benchmarks/README.md`][readme]: the scenario set follows the js-framework-benchmark keyed table, every scenario verifies the expected DOM state before its duration counts, and the user-facing metrics — time to first render, click throughput, transfer size, heap — are first-class alongside it.

## leptoskit, React Native, Lynx, and Flutter

If you are choosing a way to ship one interface to phones, desktops, and the web, the four serious options make different bets. They differ in where your code runs, what draws the pixels, and what crosses between the two on every update. This section compares leptoskit with the latest releases of React Native, Lynx, and Flutter in two ways:

- **How each one is built.** A table of the architecture: how messages cross from your code to the screen, and what each framework supports.
- **What each one measures.** The same app is built idiomatically in every framework, run through the same scenarios on every platform, and timed from screen recordings. That way, a framework's own frame counter never decides the result.

All four apps use the same contract. Each one shows a small coloured square whose colour marks when a scenario starts and finishes. The recording shows which frames actually reached the screen, and when. The method, the scenarios, and every app's source are in [`benchmarks/arena`][arena].

### How they are built

|                                                 | leptoskit                                                                                                                                                                  | React Native 0.88                                                                                                                             | Lynx (ReactLynx 0.129)                                                                                    | Flutter 3.47                                                                                                         |
| ----------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| Your interface code                             | Rust, compiled to WebAssembly and run by an interpreter (wasmi) on device                                                                                                  | JavaScript, compiled to Hermes bytecode                                                                                                       | JavaScript in PrimJS. React runs on a background thread; small "main-thread scripts" run on the UI thread | Dart, compiled ahead of time to machine code                                                                         |
| What draws the pixels                           | The platform's own widgets: UIKit, AppKit, Android views, GTK 4, Win32. HTML on the web                                                                                    | The platform's own widgets, through the Fabric renderer. HTML on the web through react-native-web                                             | Lynx's own native elements on iOS and Android, its own renderer on desktop, and HTML on the web           | Its own renderer (Impeller) on a GPU surface. A canvas on the web                                                    |
| How an update reaches the screen                | The Wasm code sends element changes with JSON properties to a Rust host. The host diffs, lays out with yoga, and sends the platform renderer a JSON list of widget changes | React builds immutable shadow nodes in C++ through JSI. A background commit lays them out with Yoga, and the UI thread mounts the differences | The background thread sends the UI thread a batch of patch operations against precompiled templates       | Nothing crosses: Dart builds the frame and hands a layer tree to the raster thread in the same process               |
| Where your code runs, relative to the UI thread | On the UI thread                                                                                                                                                           | On its own JavaScript thread                                                                                                                  | On a background thread, apart from main-thread scripts                                                    | On the UI thread, since 3.29 merged the UI and platform threads                                                      |
| Calling native code                             | Native plugins through a binary request and response bridge                                                                                                                | TurboModules over JSI: direct C++ calls, either synchronous or returning a promise                                                            | Native modules, called from the background thread with callbacks                                          | Platform channels, which are asynchronous and encode messages in a binary format. `dart:ffi` for synchronous C calls |
| 3D                                              | None yet                                                                                                                                                                   | Through libraries: react-native-wgpu (WebGPU, which runs three.js), react-native-filament, expo-gl                                            | None: CSS 3D transforms only, and no canvas or WebGL element                                              | `flutter_gpu` (experimental, opt-in) with `flutter_scene` (pre-1.0)                                                  |
| Updates without an app store release            | Built in: signed WebAssembly code push                                                                                                                                     | Through third parties, such as Expo Updates                                                                                                   | Bundles load from wherever the host app fetches them                                                      | Not officially supported. Shorebird is a third-party service                                                         |
| Platforms                                       | Web, iOS, Android, macOS, Windows, Linux, HarmonyOS (experimental)                                                                                                         | iOS and Android. macOS and Windows through Microsoft's ports, which trail the core release. The web through react-native-web                  | iOS, Android, HarmonyOS, web. macOS and Windows since 3.7                                                 | iOS, Android, web, macOS, Windows, Linux                                                                             |

### What the arena measures

| Scenario    | What it does                                                                                                                | What it tells you                                                            |
| ----------- | --------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| Cold open   | Launches the app and waits for a ready screen of 20 rows                                                                    | How long people wait before they see your app                                |
| Big bang    | Mounts 1,000 cards, updates every tenth, then clears them, each timed to the frame that shows it                            | How fast large changes to the screen land                                    |
| Echo        | Sends 2,000 round trips each of nothing, an integer, a 1 KiB string, 64 KiB of bytes, and a 100-field object to native code | What a call between your code and native code costs, and what a payload adds |
| Ticker wall | Rewrites 300 numbers on every frame for five seconds                                                                        | Whether the path from your code to pixels keeps up with the display          |
| Autoscroll  | Scrolls a 2,000-item list from your code, one step per frame                                                                | Smoothness when your code drives the motion                                  |
| Busy hands  | Animates the list natively while your code is blocked 40 ms out of every 100                                                | Whether slow app code makes scrolling stutter                                |
| Size tiers  | Builds the same app at 1, 12, and 120 screens                                                                               | A framework's fixed cost, and what each extra screen of your app adds        |

Frame numbers are presented frames per second and **hitch time**: milliseconds of lateness per second of motion, as Apple defines it. Under 5 ms/s feels smooth and over 10 ms/s is visible stutter. Simulators, emulators, and headless browsers run on a desktop CPU without the phone's GPU timing. Their tables are marked _shape only_: they show which framework is ahead, not the numbers a phone would show.

### iOS

<!-- BEGIN GENERATED: arena-ios -->

Not measured yet.

<!-- END GENERATED: arena-ios -->

### Android

<!-- BEGIN GENERATED: arena-android -->

Not measured yet.

<!-- END GENERATED: arena-android -->

### macOS

<!-- BEGIN GENERATED: arena-macos -->

Not measured yet.

<!-- END GENERATED: arena-macos -->

### Windows

<!-- BEGIN GENERATED: arena-windows -->

Not measured yet.

<!-- END GENERATED: arena-windows -->

### Linux

<!-- BEGIN GENERATED: arena-linux -->

Not measured yet.

<!-- END GENERATED: arena-linux -->

### Web

<!-- BEGIN GENERATED: arena-web -->

Not measured yet.

<!-- END GENERATED: arena-web -->

### App size: minimal, medium, and large

Every framework builds the same three apps from one specification:

- **minimal**: one counter screen;
- **medium**: 12 screens, made up of a home menu and three each of counter, list, form, and detail screens;
- **large**: the same mix at 120 screens.

Every screen has its own labels and data, as in a real app. Navigation is a piece of state, not a routing library, so no framework pays for a dependency the others skip. "Each extra screen" is the difference between large and medium, divided by the 108 screens between them. It separates what a framework costs before you write anything from what your code adds.

<!-- BEGIN GENERATED: arena-sizes -->

Not measured yet.

<!-- END GENERATED: arena-sizes -->

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

## The view layer, the wasm runtime, and a JavaScript baseline

The browser numbers above measure leptos rendering to the DOM. On iOS and Android, leptoskit apps do something else entirely: the interface is a wasm module running in [wasmi](https://github.com/wasmi-labs/wasmi) — an interpreter, because iOS forbids JIT compilation — driving leptoskit's own view layer (`leptoskit_native` over the `leptoskit_yoga` layout engine) through a versioned ABI. This table measures those layers directly, with one binary that runs on the host, in the iOS simulator, and on an Android emulator.

Three layers, one workload — a 1,000-row list, the same shape the browser benchmark renders:

- **native** and **yoga** — the view layer on its own: build the element tree, commit it (resolve, create, lay out), relayout after a viewport change, diff a partial update and a removal, resolve Tailwind classes, dispatch events, hit-test, and the layout engine standalone (flat, grid, incremental).
- **wasm** — the shipped path: a Rust guest compiled to `wasm32-unknown-unknown` (`benchmarks/native/guest`) builds the list, handles presses, and re-renders through the real rendering ABI, interpreted by wasmi, with the host decoding JSON props and committing to the surface.
- **js** — the same guest written in JavaScript, call for call, run in [QuickJS](https://bellard.org/quickjs/): a bytecode interpreter, the same execution class as the Hermes and JavaScriptCore engines a React Native app runs on where JIT is unavailable. It drives the same ABI into the same surface, so the only variable is the layer running the app logic. QuickJS is a stand-in for that class, not a Hermes measurement.

Every scenario verifies the surface afterwards (row counts, label text), and the host side is identical for the wasm and JavaScript columns, so differences between them are guest-layer differences. One finding worth knowing before reading the numbers: a `commit()` reassembles the whole tree on the host, so an event round trip costs about the same whichever layer sent it — the per-commit assembly, not the interpreter, dominates.

<!-- BEGIN GENERATED: benchmarks-native -->

| metric                                                                  |        host |        ios |    android |
| ----------------------------------------------------------------------- | ----------: | ---------: | ---------: |
| build a 1,000-row element tree                                          |    0.226 ms |   0.788 ms |     1.2 ms |
| first commit of 1,000 rows                                              |      2.8 ms |     5.1 ms |    10.4 ms |
| dispatch 1,000 press events                                             |    0.008 ms |   0.012 ms |   0.016 ms |
| press events dispatched per second through a 3-level bubble             | 128336756/s | 81083273/s | 64279745/s |
| 1,000 hit tests across the viewport                                     |    0.935 ms |    18.1 ms |    12.9 ms |
| relayout 1,000 rows after a viewport change                             |      2.0 ms |     5.3 ms |     4.3 ms |
| remove the middle row of 1,000                                          |      1.7 ms |     5.7 ms |     4.2 ms |
| resolve Tailwind classes on 1,000 rows                                  |    0.223 ms |     1.1 ms |   0.933 ms |
| diff and patch every tenth label of 1,000 rows                          |      1.6 ms |     4.1 ms |     4.1 ms |
| lay out a 1,000-row column                                              |    0.346 ms |   0.625 ms |   0.578 ms |
| lay out 1,000 cells in a 20-column grid                                 |    0.594 ms |     1.2 ms |   0.873 ms |
| relayout 1,000 rows after one row changed height                        |    0.249 ms |   0.301 ms |   0.397 ms |
| wasm: 100 press round trips, one label updated each                     |      205 ms |     319 ms |     746 ms |
| wasm: press → guest update → commit round trips per second              |       487/s |      313/s |      134/s |
| wasm: compile and instantiate the guest module                          |    0.171 ms |   0.453 ms |   0.599 ms |
| wasm: guest rewrites all 1,000 labels and the host diffs them           |      4.9 ms |     6.4 ms |    16.4 ms |
| wasm: guest builds 1,000 rows through the ABI and the host commits them |      6.8 ms |    20.9 ms |    22.4 ms |
| js: 100 press round trips, one label updated each                       |      146 ms |     227 ms |     431 ms |
| js: press → guest update → commit round trips per second                |       683/s |      441/s |      232/s |
| js: create a QuickJS runtime and load the guest script                  |    0.124 ms |   0.153 ms |   0.314 ms |
| js: guest rewrites all 1,000 labels and the host diffs them             |      4.1 ms |     5.0 ms |    11.9 ms |
| js: guest builds 1,000 rows through the ABI and the host commits them   |      5.5 ms |     7.7 ms |    18.8 ms |

_Measured per platform — host: 2026-10-21 on macos aarch64, 12 cores, 15 runs; ios: 2026-10-20 on ios aarch64, 12 cores, 15 runs; android: 2026-10-20 on android x86_64, 2 cores, 15 runs; medians._

<!-- END GENERATED: benchmarks-native -->

The iOS and Android columns come from the iOS simulator and an Android emulator. Simulators and emulators execute on the host CPU: they verify the pipeline and expose platform behaviour, but physical-device numbers differ.

## App size

What a minimal app weighs on every platform leptoskit publishes to. Each row is the artifact a fresh `leptoskit new` scaffold produces in a release build with the default plugin set, measured by walking what actually ships: the browser's initial load (brotli, as served), the macOS and iOS application bundles, and the Android APK. The platform build tools print the same report at the end of every `leptoskit native build --release`.

<!-- BEGIN GENERATED: benchmarks-sizes -->

|                   artifact | host code | wasm guest | code push baseline |    total |
| -------------------------: | --------: | ---------: | -----------------: | -------: |
| web, initial load (brotli) |    5.7 KB |   116.8 KB |                  — | 122.5 KB |
|              macOS App.app |   2.99 MB |   116.1 KB |                  — |  3.11 MB |
|    iOS App.app (simulator) |   4.83 MB |          — |            52.9 KB |  4.88 MB |
|        Android APK (arm64) |   5.50 MB |          — |            53.1 KB |  5.64 MB |

_Measured 2026-10-22 on darwin aarch64; the scaffolded app, release builds, default plugins._

<!-- END GENERATED: benchmarks-sizes -->

The host code column is the linked binary (the Rust runtime: wasmi, the view layer, yoga, the plugins, and code push's TLS) on Apple platforms and the uncompressed `lib*.so` inside the APK on Android; the wasm guest is the app's compiled interface, carried raw on macOS and gzipped inside the `app.lkb` code push baseline on iOS and Android. Numbers are from one machine and one Xcode/NDK pair — treat them as the shape of a leptoskit app, not a promise; the committed JSON carries the exact inputs.

## Formatting

`leptoskit format` has its own benchmarks, compared against leptosfmt in the [formatting guide](guide/12-formatting.md#benchmarks): a formatted 4,178-line file in 84 ms, where rustfmt alone takes 67 ms, and an unchanged workspace of 335 files checked in 30 ms.

[readme]: https://github.com/ifiokjr/leptoskit/blob/main/benchmarks/README.md
[arena]: https://github.com/ifiokjr/leptoskit/blob/main/benchmarks/arena/README.md
