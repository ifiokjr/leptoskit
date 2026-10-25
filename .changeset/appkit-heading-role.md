---
"leptoskit_appkit": patch
---

Expose `role="heading"` to `VoiceOver` as `AXHeading` without linking `NSAccessibilityHeadingRole`, which only exists from macOS 26, so macOS apps keep launching on macOS 14 through 25.
