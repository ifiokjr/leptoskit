---
"release": minor
---

# Record source locations and commits for DevTools

With `leptoskit_native`'s `devtools` feature, every element and component records the caller's file and line. Elements keep their source through style resolution, and the surface reports each commit with mutation counts and, while profiling, layout and apply timings. Without the feature, `Element` and the surface are unchanged.
