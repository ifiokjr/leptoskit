---
"release": minor
---

# Wait for the display with `leptoskit::guest::next_frame`

Native apps can now animate from app code in step with the display. `leptoskit::guest::next_frame().await` resolves once per display frame on iOS and macOS (`CADisplayLink`) and Android (`Choreographer`). The host lets the app render inside that frame's callback, so a render that fits in a frame reaches the screen on the next vsync.

Before this, an app had to write its own frame plugin. Each frame then crossed several main-thread task hops and a paused display link, and that cost three to four frames per update.

The clock is a reserved plugin on the existing bridge, not a new WebAssembly import. A bundle that uses it still loads on apps built before it existed, where `next_frame` returns `Error::Unavailable`, so code push can deliver it safely. Hosts without a display link do the same: Windows, Linux, and HarmonyOS.

Hosts that embed `leptoskit_bridge_ffi` directly opt in with `lk_app_enable_frames`, then call `lk_app_frame` from their display link while `lk_app_frame_requested` is 1.
