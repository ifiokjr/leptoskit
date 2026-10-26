# leptoskit_tui

The terminal backend: it commits the same shared element tree every other backend commits — `View`, `Text`, `Button`, `TextInput`, sliders, scroll views, Tailwind classes and all — into an interactive terminal instead of platform widgets.

The cell grid is the widget layer. Every cell is 8×16 logical pixels, so Yoga layout, Tailwind spacing, and text measurement all land on the monospace grid your terminal already draws. Keyboard focus, a block caret, mouse clicks, and scrolling are owned by the backend, the way GTK owns its own input; they surface through the shared `NativeEvent` model every other platform uses.

Terminal apps are for long-lived interfaces — editors, dashboards, chat clients. Short-lived command-line tools belong to `clap`, not to a cell grid.

- Embed it: `NativeSurface::new(TuiBackend::new())`, drive it yourself.
- Ship it: `leptoskit::tui::run(options, || screen())` runs the whole loop.
- Generate it: `leptoskit new --platform tui` writes a runner that loads your app's wasm guest over a terminal host.
