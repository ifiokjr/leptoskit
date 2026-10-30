---
"release": minor
---

# Add native Rive vector playback

Add thread-confined native playback to `leptoskit_rive`, a pinned offline C++ runtime, and a shared `Rive` element with typed input updates. Native hosts render vector frames into platform image widgets on macOS, iOS, Android, GTK, Windows, and experimental HarmonyOS. Frames stay outside Wasm guest memory and UI JSON; idle, hidden, inactive, and reduced-motion scenes stop scheduling frames. Native playback rejects unsupported asset features and bounds encoded files, player counts, raster dimensions, and input bindings. Text, images, scripts, audio, layout, and view-model data binding are not supported by the native renderer.
