---
"release": minor
---

# Pad shared screens by the host's safe-area insets

Padding utilities accept `safe` and `safe-offset-*` values, following the `tailwindcss-safe-area` plugin: `pt-safe` pads by the top inset the host reports, and `pb-safe-offset-4` adds a spacing value to the bottom inset. Native hosts supply the insets through `NativeSurface::set_safe_area` and `StyleContext::safe_area`. `UIKitHost` forwards its controller view's `safeAreaInsets`, and the browser emits `env(safe-area-inset-*)`. Desktop windows report no insets, so a narrow desktop window no longer gets a status-bar gap meant for phones.

The gallery's phone screens use these utilities in place of fixed top and bottom padding. The Android gallery reads its window insets. Every screen now scrolls instead of clipping on short phones such as a 320×640 display.
