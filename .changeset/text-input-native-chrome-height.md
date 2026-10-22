---
"release": patch
---

# Leave room for native chrome around unstyled text inputs

An unstyled `TextInput` or `PasswordInput` with `h-auto` was exactly as tall as its text, so its frame clipped the top and bottom of the native text-field border, such as GTK's entry border and AppKit's rounded bezel. Text inputs that keep the native chrome now measure 12 layout units taller than their text, which makes an unstyled field 28 tall at the default font size, the same height `PasswordInput` already defaults to. Inputs with their own background, border, or corner radius draw no native chrome and still measure from their text and padding, and an explicit height class such as `h-5` still wins.

`NativeProps::is_custom_drawn` reports whether props replace a control's native chrome, the rule the GTK and AppKit backends already applied.
