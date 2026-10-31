---
"release": perf
---

# Measure a screen through the guest adapter

`benchmarks/native` gains a screen layer: a Rust guest that renders the arena's ticker and card screens through the real element-tree adapter, so the benchmark prices the work a shipped app does — builders, Tailwind resolution, the retained tree, and the wire — rather than a hand-written ABI sequence. It reports the interpreter instructions one frame consumes, its wall time, and the mutations a frame queues, and `WasmHost::last_fuel_used` exposes that interpreted work so a simulator or device run can attribute it.

Android's text measurer reuses one paint per size and weight and measures one-line text with `BoringLayout`, which skips line breaking. A frame that rewrites a screen of numbers now measures each label through a single-run path instead of a line breaker, and layout asks a label for several constraints, so the saving lands on every relayout.

`NativeSurface` accepts a tree the host already owns, so a renderer that builds one fresh per frame no longer clones it to keep the source.
