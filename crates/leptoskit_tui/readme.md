# leptoskit_tui

The terminal backend: it commits the same shared element tree every other backend commits — `View`, `Text`, `Button`, `TextInput`, sliders, scroll views, Tailwind classes and all — into an interactive terminal instead of platform widgets.

The cell grid is the widget layer. Every cell is 8×16 logical pixels, so Yoga layout, Tailwind spacing, and text measurement all land on the monospace grid your terminal already draws. Keyboard focus, the caret (your terminal's own cursor), mouse clicks, and scrolling are owned by the backend, the way GTK owns its own input; they surface through the shared `NativeEvent` model every other platform uses.

Terminal apps are for long-lived interfaces — editors, dashboards, chat clients. Short-lived command-line tools belong to `clap`, not to a cell grid.

- Embed it: `NativeSurface::new(TuiBackend::new())`, drive it yourself.
- Ship it: `leptoskit dev --platform tui` builds and runs your app in this terminal, and `leptoskit build --platform tui` builds the runner you ship. Embedded, `leptoskit::tui::terminal::run(options, || screen())` runs the whole loop, or `run_app` drives your own `TuiApplication`.
- See it: `examples/opencode` is a full coding-agent interface written as file routes, matched cell for cell against opencode's terminal app, and built for the terminal and the browser by the leptoskit CLI.
- Generate it: `leptoskit new --platform tui` writes a runner that loads your app's wasm guest over a terminal host.
