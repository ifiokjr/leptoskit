<h1 align="center">leptoskit_testing</h1>

<p align="center">
  <strong>leptoskit_testing</strong> runs Flutter-style widget tests over the production renderer, Yoga layout, and an injected native backend.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_testing"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_testing"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_testing.svg"></a>
  <a href="https://docs.rs/leptoskit_testing"><img alt="docs.rs" src="https://docs.rs/leptoskit_testing/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Flutter-style widget tests for Leptoskit's native element trees. The tester uses the production `NativeSurface`, Yoga layout, reconciliation, event handlers, and an injected backend. `HeadlessBackend` is the default; `AppKitBackend` renders real macOS widgets.

- Query by text, accessibility label, role plus accessible name, or a test ID set with `Element::test_id`. The `Queries` trait adds Testing Library's `get_by_*`, `get_all_by_*`, `query_by_*`, and `query_all_by_*` forms, shared with the browser crate [`leptoskit_testing_library`](../leptoskit_testing_library).
- `tap`, `toggle`, `set_slider_value`, `enter_text`, and `scroll` dispatch typed events and render the resulting state. Boolean controls propose checked values; sliders snap and clamp to their configured step grid. The owner may reject a proposal by retaining its state.
- `pump_until` has an explicit frame budget. Async tests advance their executor themselves.
- Queries exclude hidden and removed nodes. Actions reject disabled controls, ambiguous matches, unhandled events, and taps obscured at their center.
- Errors contain the committed tree, with password text and values redacted. Explicit widget observations retain controlled values for assertions. `surface()` exposes the backend for screenshots and platform assertions.

These are widget tests. Event injection does not exercise OS input routing, permission dialogs, native scrolling, or screen readers. Approximate headless text metrics are deterministic, not a pixel-perfect substitute for platform fonts.

Run the portable example on the host:

```sh
cargo build -p leptoskit_testing --example widget_suite
cargo run -p leptoskit_cli --bin leptoskit -- test run --binary target/debug/examples/widget_suite
```

See the [testing guide](https://github.com/ifiokjr/leptoskit/blob/main/docs/src/guide/08-testing.md) for browser tests, simulator/emulator execution, CI, and the platform capability matrix.

For native controls, system permission dialogs, real screenshots, and optional colored layout diagnostics, see the [native testing guide](native-testing.md). Enable the `native-ui` feature for the host-side native driver client.

## How it fits

It tests element trees without a device, using semantic finders, interactions, bounded pumping, and diagnostic trees. The CLI can run the same journeys against a real native app.

## Use it

```rust
use leptoskit_native::Element;
use leptoskit_testing::{Finder, WidgetTester};

let tester = WidgetTester::new(|| Element::text("Hello"), 390.0, 844.0)?;
tester.find(&Finder::text("Hello"))?;
# Ok::<(), leptoskit_testing::TestError>(())
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_testing)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Testing](https://ifiokjr.github.io/leptoskit/guide/08-testing.html)
