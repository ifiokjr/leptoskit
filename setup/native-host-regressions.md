# Native host regression checks

Native application code must keep worker callbacks safe and paint content under shared layout and its explicit clipping policy. These checks cover those contracts. They do not establish the complete Solana SDK application journey.

## Apple worker callbacks

`apple-tests/CallbackLifecycleTests.swift` invokes the shipping wake and release callbacks from a C++ worker thread. Both callbacks explicitly use `@Sendable`. The factory and interpreter cleanup remain on the main actor.

The original wake callback traps under Swift 6 actor checks. Correcting only wake still exposes the original release callback trap. The corrected callbacks preserve wake coalescing, cancelled waiters, joined cleanup and generation ownership. The callback checks run with strict actor checks.

## Multiline layout

`leptoskit_native/tests/text_measurement.rs` measures explicit line breaks through the public text measurer and real Yoga layout. Five regressions fail against the original implementation. The correction measures each logical line independently, including consecutive and trailing empty lines. It preserves word wrapping and font scaling. All 203 native test bodies and two doctests pass.

`leptoskit_native/tests/props_contracts.rs` retains color and native-kind wire contracts while their private implementation is reorganized for readability.

## Installed macOS text metrics

The FFI application retains its recording backend and selects the existing `AppKitTextMeasurer` on macOS. This gives layout the system font metrics used by the unchanged native renderer. The public approximate measurer remains unchanged for callers that select it.

`leptoskit_bridge_ffi/tests/macos_text.rs` starts a real interpreted guest through the application constructor and compares its Text frame with native font measurement. `controller-tests/MacTextPaintTests.swift` captures real AppKit rendering in a parent canvas, comparing every painted pixel with a tall control. It covers explicit line breaks, wrapping, scaled fonts, accents, descenders and property updates. Parent clipping and adjacent content are checked separately.

The parent canvas matters: natural glyph ink may extend beyond its line box when clipping is disabled. A bitmap cropped to the widget frame would discard that permitted ink and incorrectly report a drawing failure. Explicit parent clipping still contains it.

## Android button paint

The maintained [Android paint application](android_widget_paint/readme.md) compiles the shipping renderer directly. It measures native padding and painted pixels at the observed compact frame. Removing native button padding allows the label to paint without changing font padding or the shared frame. Its checks also cover disabled updates, local font scaling and a real touch callback.

These are off-chain client and application checks. Solana on-chain programs use Pina or the official Solana program libraries.
