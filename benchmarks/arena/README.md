# The arena: leptoskit against React Native, Lynx, and Flutter

The arena compares four ways of shipping one interface to many platforms: **leptoskit**, **React Native**, **Lynx**, and **Flutter**. They run the same workloads on iOS, Android, macOS, Windows, Linux, and the web. Each framework gets one app that drives itself through a fixed set of scenarios and reports the result in one shared format. The operating system, not the framework, decides how many frames reached the screen.

There is no open, reproducible benchmark that compares React Native's New Architecture, Lynx, and Flutter, as of October 2026. Published comparisons have three recurring problems:

- They are run by a vendor, on one flagship phone per platform.
- Their headline is average FPS.
- The apps they compare are not identical.

The arena exists to do better than that, and to give leptoskit a target worth beating.

## Ground rules

1. **One spec, four implementations.** Scenario behaviour, labels, sizes, and data come from `spec/`. Every app implements the same spec in its framework's idiomatic style. The size-tier apps are generated from `spec/app.json`, so minimal, medium, and large stay identical across frameworks.
2. **Apps drive themselves.** The harness launches an app with a scenario name. The app runs that scenario on its own clock, checks the result, and prints one `ARENA` report line. No UI automation sits in the timed path, because an Appium or XCUITest round trip would dwarf the work being measured.
3. **The OS counts the frames.** Each app shows a beacon: a 32×32 square whose colour marks the scenario's phases. The harness records the screen and drops duplicate frames, which leaves the frames the user actually saw, each with a presentation timestamp. It finds the beacon's colour changes in those frames and computes FPS, hitches, and time-to-pixel from them. This works the same way for a canvas renderer (Flutter), a dual-thread engine (Lynx), and native widgets (React Native and leptoskit). The apps' own counters are reported too, but they are never used to rank.
4. **Two lanes.** The **idiomatic** lane is what each framework's documentation teaches. The **tuned** lane uses the framework's escape hatch: Reanimated worklets or synchronous JSI for React Native, main-thread scripts for Lynx, `dart:ffi` for Flutter, and direct host calls for leptoskit. Both lanes are published. A framework is judged both on what people normally write and on what is possible when they push.
5. **Measure honestly.**
   - Use release builds only.
   - Do one unrecorded warm-up launch.
   - Interleave frameworks in every round (A B C D, then D C B A) so that thermal and background drift spreads evenly.
   - Report medians with p95, and add a bootstrap 95% confidence interval when there are enough samples.
   - A scenario counts only if its correctness check passed. A broken app reports `ok: false` and has no number.
6. **Label the hardware.** Simulators and emulators run on the host CPU and don't use device GPU timing. Their frame numbers are marked _shape only_. Physical-device runs record the model, OS, and refresh rate.

## What is measured

| Area      | Scenario     | Name                   | What it shows                                                                                                                                                                                                                                                                                               |
| --------- | ------------ | ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Rendering | `cold`       | Cold open              | Process launch to the first presented frame showing the ready screen, measured from the recording. Includes runtime start, bundle or wasm load, first layout, and first paint.                                                                                                                              |
| Rendering | `mount`      | Big bang               | Mount 1,000 cards, update every tenth, then clear them. Each step is timed from the state change to the presented frame. This is js-framework-benchmark's keyed table, on device.                                                                                                                           |
| Bridge    | `echo`       | Echo ladder            | Round trips to a native echo module with five payloads: nothing, an integer, a 1 KiB string, 64 KiB of bytes, and a 100-field object. Reports p50 and p99 latency and calls per second, for both the sync and the async tier where the framework has both.                                                  |
| Frames    | `ticker`     | Ticker wall            | 300 tiles, each rewritten by app logic on every frame for 5 seconds, with no native animation driver. This is the full path from logic to renderer to pixels. Reports presented FPS and hitch time per second.                                                                                              |
| Frames    | `autoscroll` | Autoscroll             | A 2,000-item feed scrolled by app logic at a constant 1.5 pt/ms, one offset per frame. Reports presented FPS and hitch time per second.                                                                                                                                                                     |
| Frames    | `busy`       | Busy hands             | The feed animates natively to its end while the logic thread blocks for 40 ms out of every 100 ms. It shows whether scrolling is isolated from app logic, which is where these architectures differ most: native scrolling (React Native, Lynx, leptoskit) against a UI thread shared with build (Flutter). |
| Size      | `tiers`      | Minimal, medium, large | Three generated apps with identical behaviour (1, 12, and 120 screens), measured as release artifacts on every platform. The marginal cost of one screen, (large − medium) / 108, separates a runtime's fixed floor from what each line of app code costs.                                                  |

The research behind these choices is summarised in [Methodology notes](#methodology-notes).

### Planned scenarios

| Scenario   | Name           | What it adds                                                                                                                                                                                                                                                |
| ---------- | -------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `firehose` | Firehose       | Native code emits events at 60, 240, 1,000, and 5,000 Hz, and the app renders the latest value. Reports the share delivered, the latency from last event to pixel, and frames dropped. This is the native-to-logic direction, which most benchmarks ignore. |
| `knee`     | The knee       | Ramps the number of prop updates per frame until frames drop, and reports the knee: how much UI work fits in one 60 Hz frame.                                                                                                                               |
| `puppet`   | Puppet         | A card follows an injected drag through app logic. Reports how far the card lags the finger, in frames, read from the recording.                                                                                                                            |
| `spin`     | Spin up        | Lit, textured cubes. Ramps N and reports the largest N held at the display's refresh rate, plus a 3D view composited under native UI.                                                                                                                       |
| `marathon` | Marathon       | A 10-minute loop of navigation and scrolling. Reports the memory slope (MB per minute) and the peak.                                                                                                                                                        |
| `patch`    | One-line patch | The bytes an over-the-air update downloads to change one string: leptoskit code push against a Hermes bundle, a Lynx bundle, and Shorebird for Flutter.                                                                                                     |

## The app contract

Every arena app, in every framework, implements this contract. A harness that only knows the contract can measure any of them.

### Launching

The scenario arrives as `scenario=<id>`, passed the way each platform passes arguments:

| Platform              | How                                                                                                         |
| --------------------- | ----------------------------------------------------------------------------------------------------------- |
| iOS                   | Launch argument: `xcrun simctl launch <udid> <bundle> --scenario <id>`, or `devicectl … -- --scenario <id>` |
| Android               | Intent extra: `adb shell am start -S -W -n <component> --es scenario <id>`                                  |
| macOS, Windows, Linux | Command-line argument `--scenario <id>`                                                                     |
| Web                   | Query string `?scenario=<id>`                                                                               |

A tuned lane that needs its own launch, such as Lynx's main-thread ticker, also takes `lane=tuned`, passed the same way: `--lane tuned`, `--es lane tuned`, or `?lane=tuned`. The app's `arena.json` lists those scenarios under `lanes`. A tuned lane reported inside the idiomatic launch needs no entry; the synchronous echo tier is one example.

With no scenario, the app shows a home screen with one button per scenario, so a person can run any of them by hand.

### The screen

The root is a column that fills the safe area. Its first child is a 48 pt **header row** containing two things:

- **The beacon**: a 32×32 pt square with 8 pt margins and no rounding, always at the top-left of the safe area. Its background colour is the scenario's phase:

  | Phase   | Colour    | Meaning                                                            |
  | ------- | --------- | ------------------------------------------------------------------ |
  | idle    | `#000000` | Launched, and the scenario has not started                         |
  | running | `#FF0000` | Set in the same frame as the first measured change                 |
  | done    | `#00FF00` | Set in the same frame as the last measured change, after the check |
  | failed  | `#0000FF` | The correctness check failed                                       |

  The `cold` scenario shows `done` as soon as its ready screen is presented. `mount` cycles running then done for each step, with an idle frame in between.
- **The status text**: `<scenario> <phase>`. A person watching can read it. The harness doesn't parse it.

Screen backgrounds are `#FFFFFF`, and text is `#111111` in the platform's default sans-serif font. Nothing else on screen may use the four beacon colours.

### The report line

When a scenario finishes, the app writes exactly one line to its standard log:

```text
ARENA {"v":1,"framework":"flutter","frameworkVersion":"3.47.8","platform":"ios","scenario":"echo","lane":"idiomatic","ok":true,"metrics":{...},"samples":{...}}
```

| Platform          | Where the line goes                                                         |
| ----------------- | --------------------------------------------------------------------------- |
| iOS and macOS     | `NSLog` or `print`, read with `simctl launch --console-pty` or `log stream` |
| Android           | Logcat, tag `ARENA`                                                         |
| Windows and Linux | Standard output                                                             |
| Web               | `console.log`                                                               |

The fields:

- `metrics`: medians and summary numbers, keyed in snake case with the unit as a suffix (`_ms`, `_us`, `_per_s`, `_bytes`, `_count`).
- `samples`: the per-iteration values behind each median, under the same keys, at most 100 values per key. The harness recomputes medians from these, so the app's own median is never trusted. Where a scenario has more iterations than that (`echo` times 2,000 calls per payload), the app computes the percentiles from its full set in `metrics`, and `samples` holds the mean of each consecutive batch of 100 calls.
- `error`: on failure, the reason.
- `notes`: a list of strings recording what a framework could not do as specified, such as "64 KiB bytes are passed as base64".

System logs truncate long lines: `os_log` at about 1 KB, and logcat at about 4 KB. A report whose JSON is longer than 800 bytes is therefore split into chunks of at most 800 bytes. Each chunk is written as its own consecutive line, `ARENA-PART <index>/<count> <chunk>`, with a 1-based index. The harness joins them back together. A short report stays a single `ARENA` line.

### Scenarios

All timings use a monotonic clock: `performance.now()`, `Stopwatch`, `CACurrentMediaTime`, `SystemClock.elapsedRealtimeNanos`, or `Instant`.

**`cold`**

Render a ready screen with the header and 20 text rows (`Row 1` … `Row 20`). Set the beacon to done in the first frame that shows them. Report `first_frame_ms` if the framework exposes it relative to process start. The harness measures launch to presented frame itself.

**`mount`**

A non-virtualised scroll view, initially empty. Run five rounds, with one idle frame between steps. Each round:

1. Create 1,000 cards. Card `i` contains the text `Card i` and a second text `Subtitle i`.
2. Update every tenth card's title to `Card i !`.
3. Clear all cards.

Time each step from the state change to the framework's earliest after-frame hook. Before the beacon turns done, check the count of rendered rows and the text of the first and last rows. Report `create_ms`, `update_ms`, and `clear_ms`.

**`echo`**

A native module `ArenaEcho`, written in the platform language. Each payload is sent 200 times as a warm-up, then 2,000 times sequentially and timed, awaiting each call. Verify every result.

| Payload    | Call                                                                                                                      |
| ---------- | ------------------------------------------------------------------------------------------------------------------------- |
| `none`     | `echo()` returns nothing                                                                                                  |
| `int`      | `addOne(41)` returns 42                                                                                                   |
| `str1k`    | Echo a 1,024-character ASCII string                                                                                       |
| `bytes64k` | Echo 65,536 bytes where byte `i` is `i % 251`. Use base64 only if the framework cannot pass binary, and say so in `notes` |
| `obj`      | Echo `{f0: 0, …, f99: 99}`                                                                                                |

The **async tier** is the idiomatic call: a Promise TurboModule, a `MethodChannel`, a Lynx NativeModule callback, or a leptoskit plugin call. The **sync tier**, where it exists, is the tuned lane: a synchronous JSI or TurboModule method, `dart:ffi`, or a direct host import. Report `<tier>_<payload>_p50_us`, `<tier>_<payload>_p99_us`, and `<tier>_<payload>_per_s`.

**`ticker`**

A 15 × 20 grid of 300 tiles. On every frame for 5,000 ms, set tile `i`'s text to `frame + i`, through the framework's normal state path. Use the framework's frame callback: `requestAnimationFrame`, `Ticker` or `scheduleFrameCallback`, or the equivalent. Report:

- `frames_count`: the logic updates the app applied.
- `skipped_count`: frame callbacks that arrived more than 1.5 vsyncs late.
- `final_frame`: verified as the value tile 0 shows on screen.

**`autoscroll`**

A virtualised feed of 2,000 items, using the framework's idiomatic list: `FlatList`, `ListView.builder`, Lynx `<list>`, or leptoskit's virtual list. Item `i` is a card 72 pt high with three parts:

- the title `Item i`;
- the body `This is the body of item i.` repeated twice;
- a 40×40 square whose colour is `hsl(i * 37 % 360, 60%, 50%)`.

On every frame, scroll without animation to `offset += 1.5 × dt` for 5,000 ms. Report `frames_count` and `final_offset`.

**`busy`**

The same feed. Start the framework's native animated scroll to item 1,500. While it runs, block the logic thread for 40 ms of every 100 ms. Report `blocks_count`. The harness reads the presented frames.

## Size tiers

`spec/app.json` holds the generator inputs. `scripts/arena/tiers.ts` expands them deterministically into a list of screens, and each framework's emitter writes them into its tier project as ordinary source files: one file per screen, plus a navigator.

| Tier    | Screens | Contents                                                 |
| ------- | ------: | -------------------------------------------------------- |
| minimal |       1 | One counter screen                                       |
| medium  |      12 | Home menu and 3 each of counter, list, form, and detail  |
| large   |     120 | Home menu and 30 each of counter, list, form, and detail |

Every screen's labels and data are distinct, as in a real app, so a framework can't share code between screens that a real app couldn't. Navigation is a piece of state holding the current screen, with no routing library, so that no framework pays for a dependency the others skip. The apps use only components every framework draws natively today: views, text, buttons, text inputs, and scroll views.

Artifacts are measured as each platform ships them:

| Platform | Artifact                                                          |
| -------- | ----------------------------------------------------------------- |
| Android  | The arm64-v8a release APK, plus its uncompressed native libraries |
| iOS      | The device (arm64) Release `.app`, unsigned                       |
| macOS    | The Release `.app`                                                |
| Windows  | The release binary plus its runtime files                         |
| Linux    | The release bundle                                                |
| Web      | The bytes transferred on first load, compressed as served         |

## Platform coverage

| Framework         | iOS | Android | macOS                               | Windows                               | Linux | Web                       |
| ----------------- | :-: | :-----: | ----------------------------------- | ------------------------------------- | :---: | ------------------------- |
| leptoskit         | yes |   yes   | yes                                 | yes                                   |  yes  | yes (DOM)                 |
| React Native 0.88 | yes |   yes   | react-native-macos (trails at 0.83) | react-native-windows (trails at 0.85) |  no   | react-native-web (DOM)    |
| Lynx              | yes |   yes   | yes (own renderer, 3.7+)            | yes (own renderer, 3.7+)              |  no   | `@lynx-js/web-core` (DOM) |
| Flutter 3.47      | yes |   yes   | yes                                 | yes                                   |  yes  | yes (canvas)              |

Where an out-of-tree port trails the core release, the table records the version actually measured.

## Running it

| Task                     | What it does                                                                                                                   |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------ |
| `benchmark:arena`        | Build every app, run every scenario on every reachable target, and regenerate the docs tables                                  |
| `benchmark:arena:update` | Resolve the latest stable release of each framework, rewrite `versions.json` and the app manifests, then run `benchmark:arena` |

Both take `--frameworks`, `--platforms`, `--scenarios`, and `--runs` to narrow a run. A target whose toolchain or device is missing is skipped, and the skip is recorded in the results. Windows and Linux run on the `arena` workflow's hosted runners. Hosted runners have no GPU, so their frame numbers are _shape only_.

## Methodology notes

- **Why recordings rather than framework counters.** Flutter's `FrameTiming`, Android's `gfxinfo`, and React Native's JS FPS each see a different slice of the pipeline. `gfxinfo` misses Flutter's raster thread entirely. A recording sees what the display compositor presented, for any renderer.
  - On iOS and Android, the system recorder (`simctl io recordVideo`, `adb screenrecord`) writes variable-frame-rate video that only gets a frame when the screen changes.
  - On desktop and the web, a constant-rate capture goes through ffmpeg's `mpdecimate` to the same effect.
  - The harness uses Apple's hitch definition. A frame interval longer than one refresh period adds the excess to the hitch time, and the **hitch time ratio** is that total, in milliseconds, divided by the duration in seconds. Under 5 ms/s is good and over 10 ms/s is bad.
- **What the recording cannot see.** It cannot see frames that change nothing, which is why every frame scenario changes the screen on every frame. Its own timestamp resolution is about 1 ms. Physical-device recording costs some GPU time, and that cost falls equally on every framework.
- **Sources.**
  - js-framework-benchmark's keyed table and correctness gates.
  - Apple's hitch-time ratio (WWDC20, session 10077).
  - Android Macrobenchmark's `frameOverrunMs`.
  - Flutter's devicelab metrics: `timeToFirstFrameRasterizedMicros`, the build and raster percentiles, and `release_size_bytes` for the hello-world and gallery apps.
  - FlashList's blank-area measurement.
  - Software Mansion's 2026 KMP versus React Native study, which used one app, six devices, and framework defaults.
