---
"release": minor
---

# Run iOS and Android screens off the main thread

The apps `leptoskit new` generates for iOS and Android now run your screens' code on a thread of its own. Updating state, laying the screen out, measuring text, and preparing each change all happen there; the main thread only applies the result to UIKit or Android views. A slow update now delays only what you draw next: scrolling, animations, and touches keep running meanwhile, as they do in React Native and Lynx.

- **Rust plugins** registered as quick handlers (`rust-type`) run on that thread on iOS and Android, not the UI thread. A handler that needs UIKit or Android views belongs in Swift or Kotlin, as every bundled mobile plugin already is.
- **Plugin calls** to Swift and Kotlin handlers start in place, so a handler that answers at once replies without waiting for another turn of the main thread.
- **Rive** scenes are advanced on the same thread and their frames presented on the main one.
- The macOS app keeps running screens on the main thread, since its Rust plugins call AppKit there.
