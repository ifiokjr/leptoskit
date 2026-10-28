---
"release": minor
---

# Draw polished terminal interfaces like opencode

The terminal backend can now draw interfaces as detailed as the terminal apps people already use. `examples/opencode` proves it: a coding-agent interface matched cell for cell against opencode 2's terminal app (logo, prompt, a conversation with a split diff and shell output, a context sidebar, and a command palette over a dimmed screen), run in the terminal and, from the same element tree, in the browser.

What changed to make that possible:

- **Styled text runs.** `Element::spans` gives a `Text` runs with their own color, background, weight, and slant (`TextSpan`), which flow and wrap as one paragraph: syntax highlighting, a bold word in a sentence, a bright key beside a dim hint. The terminal paints each run and the web renders inline `<span>`s; other backends draw the joined text in the `Text`'s own style.
- **Italic.** The `italic` and `not-italic` utilities set `NativeProps::italic`, which the terminal draws with its italic attribute and the web with `font-style`.
- **Per-edge borders.** `border-t`, `border-r`, `border-b`, `border-l`, `border-x`, and `border-y`, with an optional width, set `NativeProps::border_widths`. The terminal draws light lines for one pixel and heavy ones from two, and the web draws each edge. The native widget backends leave a per-edge border undrawn.
- **Half rows.** In the terminal, a box's height keeps half a row of precision: a panel with `pt-4 pb-2` closes with a row of `▀`, and a vertical border ends in `╹`.
- **Scrims.** A translucent background dims everything painted behind it in the terminal, and an opaque one hides the glyphs it covers. Text keeps the background painted under it.
- **The real cursor.** A focused text field's caret is the terminal's own cursor, so it blinks in the user's cursor style.
- **App shortcuts and focus.** `TuiApplication::shortcut` sees every key before the focused widget, `TuiBackend::focus_id` focuses an element by its `id`, and `terminal::run_app` runs an app that keeps its own state.
- **Pinned content.** `justify-content: flex-end` and `center` now overflow the start edge when their items do not fit, as CSS does, so a conversation stays pinned to its newest message on every platform.
