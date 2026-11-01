---
leptoskit_appkit: breaking
---

# leptoskit_appkit: the macOS backend

`leptoskit_appkit` is the macOS backend: it applies leptoskit mutation batches to real `NSView`s and measures text with AppKit's own fonts.

A `NativeSurface` built on it renders the same tree the web build renders as DOM. AppKit is the reference backend, used by the gallery and the generated macOS app; the other backends follow its contract.
