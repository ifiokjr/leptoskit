# leptoskit_testing

Flutter-style widget tests for Leptoskit's native element trees. The tester uses the production `NativeSurface`, Yoga layout, reconciliation, event handlers, and an injected backend. `HeadlessBackend` is the default; `AppKitBackend` renders real macOS widgets.

```rust
use leptoskit_native::Element;
use leptoskit_testing::{Finder, WidgetTester};

let tester = WidgetTester::new(|| Element::text("Hello"), 390.0, 844.0)?;
tester.find(&Finder::text("Hello"))?;
# Ok::<(), leptoskit_testing::TestError>(())
```

- Query by text, accessibility label, widget kind plus name, or explicit test ID.
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
