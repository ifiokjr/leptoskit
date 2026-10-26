# CSS coverage evidence

Which tests back each row of the public [CSS and Tailwind coverage reference](../src/reference/css-coverage.md), and how to run them.

## Evidence by module

- [Box Model](https://www.w3.org/TR/css-box-3/): The [box tests](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_yoga/tests/css_boxes.rs) cover decoration sizes and percentage padding.
- [Sizing](https://www.w3.org/TR/css-sizing-3/): Tests cover constraints and aspect ratio with padding and borders.
- [Flexible Box Layout](https://www.w3.org/TR/css-flexbox-1/): Box tests cover fractional flex factors, constraint freezing, and reverse plus RTL.
- [Positioned Layout](https://www.w3.org/TR/css-position-3/): Tests cover signed offsets, direction-dependent edge precedence, grid-area offsets, and explicit dimensions with opposing insets.
- [Display](https://www.w3.org/TR/css-display-3/): [Component tests](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_native/tests/components.rs) cover hidden subtrees and restoration after resizing.
- [Overflow](https://www.w3.org/TR/css-overflow-3/): [AppKit tests](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_appkit/tests/appkit.rs) inspect clipping and both scroll axes. Component tests cover overflow hit testing.
- [Fonts](https://www.w3.org/TR/css-fonts-3/): Tests compare measured label sizes with actual AppKit cells.
- [Color](https://www.w3.org/TR/css-color-4/): [Compiler tests](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_native/tests/tailwind.rs) check resolved colors and palette entries.
- [Backgrounds and Borders](https://www.w3.org/TR/css-backgrounds-3/): AppKit tests cover radius clipping.
- [Syntax](https://www.w3.org/TR/css-syntax-3/) and [Values and Units](https://www.w3.org/TR/css-values-3/): Tests reject malformed brackets, non-finite numbers, invalid ranges, and unsafe tokens.
- [Media Queries](https://www.w3.org/TR/mediaqueries-4/) and [Conditional Rules](https://www.w3.org/TR/css-conditional-3/): Tests cover boundaries and environment changes.
- [Selectors](https://www.w3.org/TR/selectors-4/) (`active`): [compiler tests](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_native/tests/tailwind.rs) cover precedence against breakpoints, dark mode, and important rules, and the `:active` CSS. [Pressed-state tests](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_native/tests/pressed_state.rs) drive holds through the headless surface: in-place updates, the default dim, disabled and hidden targets, replacement, and validation. The [web tests](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_native/tests/web.rs) check the scoped `:active` rules, and the [Wasm host tests](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_wasm/tests/controls.rs) check that press phases never enter the guest. Each backend has a native check: real `NSWindow` mouse tracking in the [AppKit pressable tests](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_appkit/tests/pressable.rs), UIKit touch events in `native_probe`, the Android button's pressed state in `ProbeActivity`, xdotool presses in the [GTK widget suite](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_gtk/tests/widgets.rs), and BUTTON pointer messages in the [Windows widget suite](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_windows/tests/widgets.rs). The generated apps' Swift and Kotlin renderers are covered by the macOS controller tests and the generated-app builds. No suite drives a browser's real `:active` state; browsers apply it only to trusted pointer and key input.

## Sources, tests, and commands

The relevant source and tests are:

- [Utility parser and declaration compiler](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_native/src/tailwind).
- [Utility parsing, precedence, CSS output, and real-layout tests](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_native/tests/tailwind.rs).
- [Shared component, inheritance, responsive layout, visibility, and event tests](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_native/tests/components.rs).
- [Yoga layout tests](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_yoga/tests/layout.rs) and [CSS box tests](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_yoga/tests/css_boxes.rs).
- [AppKit view, clipping, scroll, and text measurement tests](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_appkit/tests/appkit.rs).
- [Web server-rendering tests](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_native/tests/web.rs) and [browser interaction and responsive-style tests](https://github.com/ifiokjr/leptoskit/blob/main/crates/leptoskit_native/tests/web_browser.rs).

Run the implementation checks with:

```bash
cargo test --package leptoskit_yoga
cargo test --package leptoskit_native --all-features
cargo test --package leptoskit_appkit --test appkit
```

The AppKit tests run only on macOS. Browser tests use the workspace's `wasm-bindgen-test-runner` and an installed ChromeDriver:

```bash
CHROMEDRIVER=/path/to/chromedriver cargo test --package leptoskit_native --features web --target wasm32-unknown-unknown --test web_browser
```

These suites exercise the named contracts. They are not a CSS conformance suite or evidence of every possible combination of supported declarations.
