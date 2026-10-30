---
"release": minor
---

# Scroll from code, and report scrolls in generated apps

A list can now be moved by the app, not only by the person holding it: a "jump to today" button, a chat that follows its newest message, a feed that scrolls itself.

```rust
use leptoskit::native::ScrollOffset;

ScrollView().scroll_offset(ScrollOffset::new(0.0, 800.0).animated())
```

- **`Element::scroll_offset`** scrolls a scroll view to an offset whenever the offset changes, with the platform's own animation when it is `.animated()`. Between changes the person scrolls freely. It works on the web and in the generated macOS, iOS, and Android apps; the native backend crates don't apply it yet. A non-finite offset is a `ControlTreeError::InvalidScrollOffset`.
- **The generated macOS, iOS, and Android apps report scroll events**, so `.on(NativeEventSlot::Scroll, …)` handlers run there and a `VirtualList` follows scrolling instead of staying on its first window. A scroll made from code is reported too, once it settles.
- **A scroll view's content keeps its full size.** A layout-only view placed straight inside a `ScrollView` was flattened away, so native scroll views sized their content from the children they could see. A virtual list could only scroll as far as its current window, and bottom padding on scroll content was lost.
