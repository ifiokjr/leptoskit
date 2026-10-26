# The terminal backend: design notes

The terminal backend (`crates/leptoskit_tui`) renders the shared element tree into an interactive terminal, the way Ink does for React: the same screens, committed into a cell grid instead of platform widgets. This note records the decisions an agent needs before changing it.

## Why a backend, not a component library

The value is the same as every other leptoskit platform: one screen function, rendered everywhere. A terminal-specific component library would be a second codebase. Instead the backend implements `NativeBackend` over a plain node store, exactly like GTK: the surface diffs, Yoga lays out, and the backend receives the same mutations. Terminal apps are for long-lived interfaces (Helix, Open Code); short-lived CLIs stay with `clap`.

## The cell model

One terminal cell is 8×16 logical pixels — the `ApproxTextMeasurer`'s default cell. Everything shares it:

- Yoga layout runs in logical pixels with the terminal size scaled by the cell, so Tailwind spacing lands on the grid (`p-4` = 16 px = 2 cells × 1 row).
- `TuiTextMeasurer` measures width in display columns (unicode-width) and height in wrapped lines, using the same `wrap` function the painter draws with. Measured heights and painted rows cannot disagree, and that invariant is tested.
- Pixel rects snap to cells by rounding each edge, the way the macOS backend snaps to half points. A 20-px-tall checkbox spans 1–2 rows depending on its position; that is the grid's nature.
- Font size and weight do not change measurement: the terminal's font is the user's. Weights become attributes (bold), and `font_size` is accepted and ignored. This is the one backend that deviates from `NativeProps::text_style` by design, and the deviation is documented on the measurer.

## Who owns input

Terminals are keyboard-first, and the shared tree has no key events — so the backend owns a focus ring, exactly as GTK owns its own input pipeline:

- The ring is re-derived from the committed tree per key press: document order, inherited inertness (hidden/disabled subtrees leave), positive `tab_index` first, negative removed — the HTML rules the web renderer follows.
- Tab and shift-tab always move focus, even inside a text field.
- Keys route by the focused kind: fields edit backend-owned drafts (value, caret, and the last emitted proposal, so an application echo preserves the caret and an application rewrite replaces the draft — the GTK entry contract), buttons and pressables activate on Enter and Space, radios walk their group on the arrows, sliders step their grid, scroll views scroll by a cell, a page, or the whole span. The surface normalizes proposals (`Toggle` acceptance, `SliderState::proposed_value`), so the backend emits raw events and stays dumb.
- Mouse clicks hit-test the backend's own node store (deepest node, pressable children decorative, clipped subtrees stop), focus the nearest focusable ancestor, and activate per kind, mirroring `WidgetTester::tap`. The wheel scrolls the enclosing scroll view.
- Everything surfaces through the shared `NativeEvent` model; there is no terminal-specific event type in the public API.

## Rendering and repair

`TuiBackend::render` paints the committed tree into a `Buffer` (cells with glyph, fg, bg, attributes). The loop diffs the previous buffer and writes only changed runs as ANSI escapes, quantizing colors to the terminal's depth (truecolor, 256, 16, or `NO_COLOR` with attributes intact). Terminal I/O lives behind two tiny traits (`EventSource`, `OutputSink`) so the whole session — focus, carets, scrolling, repaints — runs headlessly in tests (`leptoskit_tui::testing::{Scripted, Recorder}`).

Notable painting rules:

- The border draws after children, as GTK paints it: a border that takes no layout space still wins its edges.
- Clipping bounds one subtree only; the painter restores the outer clip for later siblings (a bug here hid everything below the first rounded card).
- Wide glyphs claim their continuation cell; text dumps skip continuations because printing the glyph already advances two columns.

## The runners

- Direct: `NativeSurface<TuiBackend>`, or `leptoskit::tui::terminal::run` for the whole loop.
- Generated apps: `leptoskit new` writes a `tui/` host crate whose runner loads the app's wasm guest over `WasmHost<TuiBackend>` — the same architecture as the Linux and Windows runners, so code push will apply when those runners gain it. The terminal runner builds on every OS; its dependency gate is `not(target_arch = "wasm32")`, not an OS.
- The gallery gains `gallery-tui` and a conformance test that snapshots all seven shared screens as text, which is the cross-backend "same screens" proof for this platform.

## Testing and coverage

`cargo llvm-cov -p leptoskit_tui` is kept at or above 90% region coverage. The only uncovered code is the real-terminal glue (raw mode, the alternate screen, crossterm's poll/read loop), which needs a tty; its escape sequences and event translations are extracted into pure functions that are tested byte for byte instead. The generated runner itself is exercised end to end in CI by building a scaffolded app for the terminal target.
