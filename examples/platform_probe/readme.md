# platform_probe

One small binary that proves the leptoskit runtime works on a target operating system, with no UI toolkit involved. The same `run_all` runs on the host, in the iOS simulator, and on an Android device or emulator, so every platform a release claims to support has the same evidence.

It runs five probes and prints one line for each:

| Probe                    | What it proves                                                                                                                                              |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `yoga:flexbox`           | The layout engine computes exact flexbox results on this target.                                                                                            |
| `yoga:grid`              | The same for CSS grid.                                                                                                                                      |
| `native:diff-and-events` | The renderer's diff and commit pipeline and event dispatch, on the headless backend.                                                                        |
| `wasm:abi-runtime`       | The on-device Wasm interpreter runs a counter app (written in WAT) through the real rendering ABI; pressing the button rewrites its label.                  |
| `codepush:delta-update`  | A signed code push release is served over real HTTP by the `leptoskit_codepush` router, downloaded as a delta, verified against a trusted key, and applied. |

A probe that panics is reported as a failure like any other, so one broken probe never hides the rest. The process exits non-zero when any fail.

```sh
cargo run -p platform_probe        # on this machine
cargo test -p platform_probe       # the same suite as an ordinary test (tests/host.rs)

devenv shell ios_sim_run           # build for the iOS simulator, install, run (needs a booted simulator)
devenv shell android_emulator_run  # build for aarch64 Android, push to the first device with adb, run
```

Expected output:

```text
PASS yoga:flexbox
PASS yoga:grid
PASS native:diff-and-events
PASS wasm:abi-runtime
PASS codepush:delta-update
platform probe: all probes passed on ios
```

The two device tasks wrap `setup/scripts/ios_probe_run.sh` and `setup/scripts/android_probe_run.sh`, which re-run themselves in a clean environment because the Nix compiler wrappers mislink simulator and Android builds. In CI the desktop suite runs in every `test` job, the iOS script runs in the `ios simulator e2e` job, and the Android job cross-compiles the probe for `aarch64-linux-android`. The widget backends have their own device verifiers; this probe covers the runtime underneath them. [Platform validation](../../docs/agents/platform-validation.md) lists what CI runs on each platform.
