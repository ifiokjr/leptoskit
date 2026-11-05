---
"release": patch
---

# Keep iOS text fields interactive

Forward optional UIKit keyboard and autofill setters through Objective-C runtime dispatch so debug iOS apps can open password fields. Tapping outside a text field in UIKitHost dismisses the keyboard without consuming control taps. The gallery Settings search field now retains typed input.
