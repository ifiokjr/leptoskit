# Terminal

In a terminal, your screens are drawn as characters: the same tree of components, laid out by the same engine, committed into your terminal's cell grid. `View` backgrounds fill cells, `Text` wraps in the monospace font your terminal already uses, buttons draw as `[ Save ]`, and a slider is a track with a knob. Nothing runs in a web view and nothing emulates a GUI: the cell grid is the widget layer.

The terminal platform is for long-lived interactive apps — editors, dashboards, chat clients — the way Ink is for React. A short-lived command-line tool still belongs to an argument parser like `clap`.

## Build and run

The terminal app builds and runs wherever cargo does, with the WebAssembly target:

```bash
rustup target add wasm32-unknown-unknown
leptoskit build --platform tui
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

- Every built-in component draws, including `Switch` as `[●]`/`[○]`, `Checkbox` as `[x]`/`[ ]`, radios, sliders, progress bars, and a braille activity spinner. `Image` draws a placeholder texture: terminals cannot fetch or decode the source.
- Keyboard focus, the block caret, drafts, and scroll offsets are owned by the backend, the way GTK owns its own input. They surface through the same `Press`, `Change`, `Submit`, `Toggle`, `ValueChange`, `Focus`, `Blur`, and `Scroll` events every other platform dispatches.
- Font sizes, line heights, and letter spacing don't change the grid; only attributes and colors do. Rounded corners, borders, and backgrounds draw with box-drawing characters and cell fills, so sub-cell precision rounds to the grid.
- The generated app runs your screens as WebAssembly, so [code push](../guide/05-code-push.md) applies to it the same way — the terminal runner accepts code push releases once the desktop runners do (today it builds with `codepush.enabled = false`, like the Linux and Windows runners).
- The terminal app builds on every platform, because every platform has a terminal.

## See more screens

The [examples](../examples.md) page shows seven screens rendered with native widgets. To see the same screens in your own terminal, clone the leptoskit repository and run the gallery:

```bash
cargo run -p gallery --bin gallery-tui settings
```

The gallery's terminal conformance test snapshots every screen as text, which is how the terminal backend stays honest about what it draws.
