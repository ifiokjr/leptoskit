---
"release": minor
---

# Build and reproduce backends from public commits

Add `leptoskit verify build`, which builds a backend from a public GitHub commit in a digest-pinned container: the commit's exact files, its locked dependencies, and two compilations with networking disabled that must produce identical executables. It writes the bundle, its inventory, and a SLSA provenance statement. `leptoskit verify reproduce` lets anyone fetch the same commit and dependencies, rebuild, and confirm the published executable byte for byte. Neither command shows what a live server runs.
