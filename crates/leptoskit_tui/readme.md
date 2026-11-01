<h1 align="center">leptoskit_tui</h1>

<p align="center">
  <strong>leptoskit_tui</strong> commits the same shared element tree into an interactive terminal cell grid, with keyboard focus, the terminal cursor, mouse input, and scrolling owned by the backend.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_tui"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_tui"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_tui.svg"></a>
  <a href="https://docs.rs/leptoskit_tui"><img alt="docs.rs" src="https://docs.rs/leptoskit_tui/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

The terminal backend: it commits the same shared element tree every other backend commits — `View`, `Text`, `Button`, `TextInput`, sliders, scroll views, Tailwind classes and all — into an interactive terminal instead of platform widgets.

The cell grid is the widget layer. Every cell is 8×16 logical pixels, so Yoga layout, Tailwind spacing, and text measurement all land on the monospace grid your terminal already draws. Keyboard focus, the caret (your terminal's own cursor), mouse clicks, and scrolling are owned by the backend, the way GTK owns its own input; they surface through the shared `NativeEvent` model every other platform uses.

Terminal apps are for long-lived interfaces — editors, dashboards, chat clients. Short-lived command-line tools belong to `clap`, not to a cell grid.

- Embed it: `NativeSurface::new(TuiBackend::new())`, drive it yourself.
- Ship it: `leptoskit dev --platform tui` builds and runs your app in this terminal, and `leptoskit build --platform tui` builds the runner you ship. Embedded, `leptoskit::tui::terminal::run(options, || screen())` runs the whole loop, or `run_app` drives your own `TuiApplication`.
- See it: `examples/opencode` is a full coding-agent interface written as file routes, matched cell for cell against opencode's terminal app, and built for the terminal and the browser by the leptoskit CLI.
- Generate it: `leptoskit new --platform tui` writes a runner that loads your app's wasm guest over a terminal host.

## How it fits

Every cell is 8×16 logical pixels, so Yoga layout, Tailwind spacing, and text measurement land on the monospace grid. `leptoskit dev/build --platform tui` drives it, and `examples/opencode` is a full coding-agent interface built with it.

## Use it

```rust,ignore
use leptoskit_native::NativeSurface;
use leptoskit_tui::TuiBackend;

let surface = NativeSurface::new(TuiBackend::new());
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_tui)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Terminal](https://ifiokjr.github.io/leptoskit/platforms/tui.html)
