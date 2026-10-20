---
"release": patch
---

# Benchmark the native view, wasm, and JS layers on device

`benchmarks/native` adds `leptoskit_bench`: one binary measuring `leptoskit_native` and `leptoskit_yoga` (build, commit, relayout, diff, Tailwind resolution, dispatch, hit testing), the wasm runtime executing a Rust-compiled guest through the real rendering ABI in wasmi, and the same guest written in JavaScript running in QuickJS as an interpreter-class baseline. It runs unchanged on the host, in the iOS simulator, and on an Android emulator through `leptoskit test run`, and emits the same report shape as the browser harness, so the pull-request comment now carries a native-layer section and the Benchmarks chapter a per-platform table. A `device comparison` workflow runs the simulator and emulator on demand.
