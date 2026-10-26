---
"release": minor
---

# file_routes everywhere; apps draw pressables

`examples/file_routes` is now a standalone workspace in the shape `leptoskit new` produces, so `leptoskit build --platform ios` — and every other destination — builds and runs it; CI builds it for each platform, launches it on an iOS simulator and an Android emulator, and drives its router through the interpreter.

Generated macOS, iOS, and Android apps now draw `Pressable` in addition to the six components they already supported, so file-router links work in generated apps.

The native Wasm runtime also resolves wasm-bindgen's linked-but-unused runtime imports, so guests whose dependency graphs pull in wasm-bindgen without calling a browser API — like `reactive_graph`, which the file router links — instantiate instead of failing with an unresolved-import error. Real browser imports still fail loudly.
