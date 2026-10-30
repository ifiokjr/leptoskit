---
"release": feat
---

# Move a scroll container from app state, and hear where it went

`ScrollView` gained a scroll command: `.scroll(x, y)` snaps the content to an
offset and `.scroll_animated(x, y)` hands the move to the platform so it keeps
moving while the app is busy. The command is applied when its value changes, so
content a person scrolled stays where they left it; a screen that wants the
container to follow its own state listens for `NativeEvent::Scroll` and stores
the offset it wants shown.

The generated iOS, Android, and macOS apps now report `scroll` events to the
guest. Before this, `leptoskit_virtual`'s list could not know where it was on
those platforms, because only the backend crates reported scrolling and the
generated apps' renderers did not. A programmatic move is reported once the
platform has clamped it, on the turn after the batch, so the guest hears where
the content actually landed rather than where the app asked it to go.

Every renderer applies the command: UIKit and AppKit scroll views through
`setContentOffset` and the clip view, Android through `scrollTo` and
`smoothScrollTo` once its document is laid out, the Rust `leptoskit_uikit` and
`leptoskit_appkit` backends, and the web adapter, which passes the offset to the
element's own `scrollTo` and remembers the value it applied on the element.

The arena's `autoscroll` and `busy` scenarios remain marked unsupported: the
command and the events reach the platform, but the arena's feed reports a
zero-height scroll container on Android, and `busy` needs the guest off the UI
thread for a platform animation to run while app logic blocks.
