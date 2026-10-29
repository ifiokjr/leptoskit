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
- Every key is offered to the application first, as `NativeEvent::Key` dispatched at the focused widget (or the root) and bubbling through its ancestors; `Element::on_key` handlers that return `Handled` keep it from focus routing, so a root view can take ctrl+p while a field is focused. `TuiApplication::dispatch` reports the outcome so the host knows. The same event reaches wasm guests (ABI event kind 8) and the web renderer's `keydown`.
- `Element::focused(true)` is controlled focus: after each commit the backend moves focus to the last focusable widget asking for it, queueing the `Blur`/`Focus` events in `TuiBackend::take_events`, which the host dispatches after each event (`TuiHost::settle`). Closing a dialog whose field asked for focus hands it back to the next widget still asking.
- The caret is the terminal's own cursor: the painter records it on the `Buffer` (`set_cursor`), and `FrameWriter` moves and shows it after every changed frame, or hides it. Focused buttons and marks still draw reverse video.
- Keys route by the focused kind: fields edit backend-owned drafts (value, caret, and the last emitted proposal, so an application echo preserves the caret and an application rewrite replaces the draft — the GTK entry contract), buttons and pressables activate on Enter and Space, radios walk their group on the arrows, sliders step their grid, scroll views scroll by a cell, a page, or the whole span. The surface normalizes proposals (`Toggle` acceptance, `SliderState::proposed_value`), so the backend emits raw events and stays dumb.
- Mouse clicks hit-test the backend's own node store (deepest node, pressable children decorative, clipped subtrees stop), focus the nearest focusable ancestor, and activate per kind, mirroring `WidgetTester::tap`. The wheel scrolls the enclosing scroll view.
- Everything surfaces through the shared `NativeEvent` model; there is no terminal-specific event type in the public API.

## Rendering and repair

`TuiBackend::render` paints the committed tree into a `Buffer` (cells with glyph, fg, bg, attributes). The loop diffs the previous buffer and writes only changed runs as ANSI escapes, quantizing colors to the terminal's depth (truecolor, 256, 16, or `NO_COLOR` with attributes intact). Terminal I/O lives behind two tiny traits (`EventSource`, `OutputSink`) so the whole session — focus, carets, scrolling, repaints — runs headlessly in tests (`leptoskit_tui::testing::{Scripted, Recorder}`).

Notable painting rules:

- A glyph written with the default background is transparent: it keeps the background painted under it. An opaque fill clears glyphs as well as backgrounds, so overlays hide what they cover.
- Translucent backgrounds (alpha below 255) tint every cell under them, glyph and background, toward the color; opencode's scrim is black at alpha 150. Default terminal colors cannot be mixed and stay as they are.
- Backgrounds and borders keep half a row of height precision (`Rows`): the top snaps to a whole row like text, and a height that ends mid-row paints its last row as `▀` in the box color, with vertical edges ending in `╹`/`╵`. Positions do not get half rows, because centering would otherwise put `▄` above every odd-centered panel.
- Border edges come from `NativeProps::border_edges`: one pixel draws light lines, two or more heavy. A border takes no cell of its own (its pixels round away), so content beside it needs padding — `border-l-2 pl-1.5` puts the panel one cell right of the bar.
- `Text` with `spans` wraps the joined text with `wrap_ranges` and paints each glyph in its run's style; measurement already uses the joined `text`, so heights agree. A no-break space wraps like a letter and prints as a blank, which is how code keeps its indentation.

- The border draws after children, as GTK paints it: a border that takes no layout space still wins its edges.
- Clipping bounds one subtree only; the painter restores the outer clip for later siblings (a bug here hid everything below the first rounded card).
- Wide glyphs claim their continuation cell; text dumps skip continuations because printing the glyph already advances two columns.

## Fidelity against a real terminal app

The opencode example (`examples/opencode`) was built against opencode 2.0.15 captured through a pty and a vt100 parser, comparing glyph, colors, and attributes cell by cell, then screenshotted in one terminal renderer (vhs) for a pixel diff. Home matched every cell and pixel; session and palette differed only in opencode's measured run time. Two layout rules came out of that work: Yoga's `justify-content: flex-end` and `center` now overflow the start edge like CSS (a chat pinned to its newest message), and opencode's split diff begins when the conversation column is wider than 120 cells. To repeat the comparison, run both apps in the same pty size with the same keystrokes and diff the vt100 screens; set `COLORTERM=truecolor` in recorders like vhs, which otherwise make `ColorDepth::detect` choose the 256-color palette.

## The runners

- Direct: `NativeSurface<TuiBackend>`, or `leptoskit::tui::terminal::run` for the whole loop.
- Generated apps: `leptoskit new` writes a `tui/` host crate whose runner loads the app's wasm guest over `WasmHost<TuiBackend>` — the same architecture as the Linux and Windows runners, so code push will apply when those runners gain it. The terminal runner builds on every OS; its dependency gate is `not(target_arch = "wasm32")`, not an OS.
- The gallery gains `gallery-tui` and a conformance test that snapshots all seven shared screens as text, which is the cross-backend "same screens" proof for this platform.

## Testing and coverage

`cargo llvm-cov -p leptoskit_tui` is kept at or above 90% region coverage. The only uncovered code is the real-terminal glue (raw mode, the alternate screen, crossterm's poll/read loop), which needs a tty; its escape sequences and event translations are extracted into pure functions that are tested byte for byte instead. The generated runner itself is exercised end to end in CI by building a scaffolded app for the terminal target.
