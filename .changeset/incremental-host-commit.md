---
"release": patch
---

# Commit only what the guest changed

A native app's commit no longer rebuilds the whole interface. The Wasm host records which nodes the guest changed since its last commit, and the renderer resolves and diffs only those parts, so updating one label in a 1,000-row list takes about 0.08 ms instead of 1.15 ms on an M-series Mac. Trees that use ids, `aria-labelledby` or `aria-describedby`, radio groups, or live regions still commit in full, because those resolve across the whole tree.

`NativeSurface::try_patch` is the new entry point for hosts that track their own changes.
