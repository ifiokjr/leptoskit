---
"release": minor
---

# Render shared screens in an interactive terminal

Add `leptoskit_tui` and the `leptoskit` facade's `tui` feature: the same element tree and Tailwind classes every platform renders, committed into the terminal's cell grid. Layout runs on the shared 8×16-px cell so measured heights are painted rows, every built-in component draws (buttons as `[ Save ]`, switches as `[●]`, sliders, progress, a braille spinner), and colors quantize to the terminal's depth down to `NO_COLOR`. Keyboard focus, the block caret, field drafts, and scroll offsets are owned by the backend and surface through the shared `NativeEvent` model, so a terminal app is the same screens with the same handlers — the Ink model, for long-lived apps rather than short-lived CLIs.

`leptoskit new` now generates a `tui/` runner that loads the app's Wasm guest over the terminal backend, and `leptoskit build --platform tui` builds it on every operating system. The gallery gains `gallery-tui` plus a conformance test that snapshots all seven shared screens as text, and the terminal backend keeps at least 90% region coverage with fully headless sessions.
