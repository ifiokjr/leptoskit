---
"release": minor
---

# Draw polished terminal interfaces like opencode

The terminal backend can now draw interfaces as detailed as the terminal apps people already use. `examples/opencode` proves it: a coding-agent interface matched cell for cell against opencode 2's terminal app (logo, prompt, a conversation with a split diff and shell output, a context sidebar, and a command palette over a dimmed screen), written as file routes and built by the leptoskit CLI for the terminal and the browser.

What changed to make that possible:

- **Styled text runs.** `Element::spans` gives a `Text` runs with their own color, background, weight, and slant (`TextSpan`), which flow and wrap as one paragraph: syntax highlighting, a bold word in a sentence, a bright key beside a dim hint. The terminal paints each run and the web renders inline `<span>`s; other backends draw the joined text in the `Text`'s own style.
- **Italic.** The `italic` and `not-italic` utilities set `NativeProps::italic`, which the terminal draws with its italic attribute and the web with `font-style`.
- **Per-edge borders.** `border-t`, `border-r`, `border-b`, `border-l`, `border-x`, and `border-y`, with an optional width, set `NativeProps::border_widths`. The terminal draws light lines for one pixel and heavy ones from two, and the web draws each edge. The native widget backends leave a per-edge border undrawn.
- **Half rows.** In the terminal, a box's height keeps half a row of precision: a panel with `pt-4 pb-2` closes with a row of `▀`, and a vertical border ends in `╹`.
- **Scrims.** A translucent background dims everything painted behind it in the terminal, and an opaque one hides the glyphs it covers. Text keeps the background painted under it.
- **The real cursor.** A focused text field's caret is the terminal's own cursor, so it blinks in the user's cursor style.
- **Keyboard shortcuts.** `Element::on_key` hears every key pressed while focus is inside the element, before the focused field acts on it, as `NativeEvent::Key(KeyPress)` with web key names. The terminal host offers each key to the app first, wasm guests receive it (ABI event kind 8), and the web renderer delivers `keydown`, keeping a handled key from the browser. `TuiApplication::dispatch` now reports the outcome.
- **Controlled focus.** `Element::focused(true)` asks for keyboard focus: the terminal moves it after each commit and the web after each render, and focus returns to the next widget still asking when a dialog closes.
- **Arbitrary breakpoints.** `min-[968px]:` and `max-[968px]:` variants, resolved natively and as media queries.
- **The viewport in guests.** A guest that exports `leptoskit_viewport(width, height)` hears the host's viewport before `leptoskit_main` and on every resize; `leptoskit::guest::viewport()` reads it.
- **Guests that link browser bindings load.** wasm-bindgen placeholder imports, such as the `console.warn` that reactive_graph (and so the file router) links, resolve to stubs that trap only if called, so routed apps run in native guests.
- **Pinned content.** `justify-content: flex-end` and `center` now overflow the start edge when their items do not fit, as CSS does, so a conversation stays pinned to its newest message on every platform.
- **The CLI builds and runs terminal apps.** `leptoskit dev --platform tui` builds the terminal app and runs it here. Client-rendered apps build without a server package, a crate with `web` and `native` features is one app for both, and a platform host may be a Cargo workspace of its own. The generated terminal host's dependencies are gated on the architecture alone.
- **No more Trunk.** The gallery's and the opencode example's browser versions build and serve with `leptoskit dev`.
