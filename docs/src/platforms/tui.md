# Terminal

In a terminal, your screens are drawn as characters: the same tree of components, laid out by the same engine, committed into your terminal's cell grid. `View` backgrounds fill cells, `Text` wraps in the monospace font your terminal already uses, buttons draw as `[ Save ]`, and a slider is a track with a knob. Nothing runs in a web view and nothing emulates a GUI: the cell grid is the widget layer.

The terminal platform is for long-lived interactive apps — editors, dashboards, chat clients — the way Ink is for React. A short-lived command-line tool still belongs to an argument parser like `clap`.

## Build and run

The terminal app builds and runs wherever cargo does, with the WebAssembly target. While you work on it, build and run it in one step:

```bash
rustup target add wasm32-unknown-unknown
leptoskit dev --platform tui
```

That builds your app and runs it in the terminal you typed it in; quit with `Ctrl+C` and run it again after a change. To build the app you ship, use `leptoskit build`:

```bash
leptoskit build --platform tui --release
.leptoskit/tui/app/your_app
```

The build compiles your app to WebAssembly and links it into a small terminal runner for your machine, the same architecture as the Linux and Windows runners. `Tab` moves focus, `Enter` presses, the arrows scroll and step sliders, the mouse clicks and scrolls where the terminal reports it, and `Ctrl+C` exits. The `tui/` folder is yours, like every host folder.

You can also embed the backend directly, the way the gallery does:

```rust
use leptoskit::native::Element;
use leptoskit::native::NativeSurface;
use leptoskit::tui::TuiBackend;

let mut surface = NativeSurface::new(TuiBackend::new());
surface.set_viewport(80.0 * 8.0, 24.0 * 16.0);
surface.set_root(&Element::text("Hello, terminal"));

let frame = surface.backend().render(80, 24);
assert_eq!(frame.to_plain_lines()[0].trim_end(), "Hello, terminal");
```

Or hand the whole loop over with `leptoskit::tui::terminal::run`.

## How the cell grid works

One terminal cell is 8×16 logical pixels, so layout is shared exactly: `p-4` is sixteen pixels, which is two cells wide and one row tall, and the text measurer wraps with the same algorithm the painter draws with. A label's measured height is always the number of rows it paints.

The terminal's font is the user's own: font sizes and weights map to attributes (bold) and colors rather than to different metrics. Colors quantize to what the terminal has — truecolor, the 256-color palette, or sixteen colors — and `NO_COLOR` turns them off without losing attributes.

## How it differs from the other platforms

- Built-in controls draw, including `Switch` as `[●]`/`[○]`, `Checkbox` as `[x]`/`[ ]`, radios, sliders, progress bars, and a braille activity spinner. `Image` and `Vector` draw a placeholder texture: the terminal cannot fetch or decode an image source, and a drawing does not resolve to cells. `Rive` playback is unsupported.
- Keyboard focus, drafts, and scroll offsets are owned by the backend, the way GTK owns its own input. They surface through the same `Press`, `Change`, `Submit`, `Toggle`, `ValueChange`, `Focus`, `Blur`, and `Scroll` events every other platform dispatches. A focused text field's caret is your terminal's own cursor, so it blinks in your cursor style and input methods compose where you type.
- Font sizes, line heights, and letter spacing don't change the grid; only attributes and colors do. Bold and `italic` draw as terminal attributes.
- Backgrounds fill cells. A box always starts on a whole row, but its height keeps half a row: a panel with `pt-4 pb-2` ends in a row of `▀` that covers only the top half of its last cell, the way polished terminal apps close their panels. Horizontal positions round to whole cells.
- Borders draw with box-drawing characters: light lines for one pixel, heavy lines from two pixels up, so `border-l-2` is a heavy `┃` bar down the left edge that ends in `╹` on a half row. A border takes no cell of its own, so give the content beside it padding.
- A translucent background, such as `bg-black/60`, dims everything painted behind it — glyphs and backgrounds alike — the way a scrim dims the screen behind a dialog. An opaque background hides what is behind it.
- The generated app runs your screens as WebAssembly, so [code push](../guide/05-code-push.md) applies to it the same way — the terminal runner accepts code push releases once the desktop runners do (today it builds with `codepush.enabled = false`, like the Linux and Windows runners).
- The terminal app builds on every platform, because every platform has a terminal.

## Styled text

A label often needs more than one style: syntax-highlighted code, a bold word in a sentence, a bright key next to a dim hint. Give a `Text` styled runs instead of a string, and they flow and wrap together as one paragraph:

```rust
use leptoskit::native::Color;
use leptoskit::native::TextSpan;
use leptoskit::prelude::Text;

let key = Color::hex("#eeeeee").unwrap();
let hint = Color::hex("#808080").unwrap();

let label = Text("").spans([
	TextSpan::new("ctrl+p ").color(key),
	TextSpan::new("commands").color(hint),
]);
```

The terminal paints each run in its own color, background, weight, and slant, and the browser renders each as an inline `<span>`. The other platforms draw the runs joined together in the `Text`'s own style, so the words are always there.

## Keyboard shortcuts and focus

Terminal apps live on the keyboard. Two shared features keep that in the same element tree on every platform:

```rust
use leptoskit::native::KeyPress;
use leptoskit::prelude::*;

let screen = View()
	.on_key(|key: &KeyPress| {
		if key.is("p", true, false, false) {
			// open the command palette
			EventOutcome::Handled
		} else {
			EventOutcome::Unhandled
		}
	})
	.child(TextInput().aria_label("Prompt").focused(true));
```

- `on_key` hears every key pressed while focus is inside the element, before the focused field acts on it, so a screen can take `Ctrl+P` while a text field has focus. Return `EventOutcome::Handled` to keep the key from the field. Keys are named the way the web names them: `"p"`, `"Enter"`, `"Escape"`, `"Tab"`, `"ArrowUp"`.
- `focused(true)` asks for keyboard focus: after each change, focus moves to that widget if it is somewhere else. Keep it on your prompt, and move it to a dialog's search field while the dialog is open; when the dialog closes, focus returns to the prompt.

Both work in the terminal and in the browser, where a handled key also skips the browser's own meaning, such as printing on `Ctrl+P`. The other platforms do not deliver key presses or move focus for you yet.

A screen that needs the terminal's size, such as a list that shows half the rows, can read it with `leptoskit::guest::viewport()` once the runner has reported it; layout and responsive classes such as `min-[968px]:flex` already follow the size on their own.

## A whole app

The opencode example in [`examples/opencode`](https://github.com/ifiokjr/leptoskit/tree/main/examples/opencode) is a whole coding-agent interface — a logo, a prompt, a conversation with a diff and shell output, a sidebar, and a command palette over a dimmed screen — written once as [file routes](../guide/10-file-routes.md) and run in both your terminal and your browser. Every reply is scripted, so it runs offline:

```bash
cd examples/opencode
leptoskit dev --platform tui    # in this terminal
leptoskit dev                   # in the browser
```

Type a prompt and press `Enter`, open the palette with `Ctrl+P`, and switch agents with `Shift+Tab`.

## See more screens

The [examples](../examples.md) page shows seven screens rendered with native widgets. To see the same screens in your own terminal, clone the leptoskit repository and run the gallery:

```bash
cargo run -p gallery --bin gallery-tui settings
```

The gallery's terminal conformance test snapshots every screen as text, which is how the terminal backend stays honest about what it draws.
