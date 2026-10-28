---
"release": patch
---

# Report scroll events from the AppKit backend

`leptoskit_appkit` scroll views now deliver `NativeEvent::Scroll` with the content offset whenever their content moves, as the iOS, Android, Linux, and Windows backends already do, so a `ScrollView`'s `.on(NativeEventSlot::Scroll, …)` handler runs on macOS and a `VirtualList` follows scrolling there. A deleted scroll view stops reporting. The generated macOS app still draws through its own renderer and does not report scroll events yet.
