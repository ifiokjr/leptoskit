---
"leptoskit_native": minor
---

# CSS color variables and a Theme map

Arbitrary color utilities can use `var(--name)` or `var(--name, fallback)`. The web stylesheet keeps the CSS function. Native paint resolves the name through `Theme::set_color` (or the fallback when the theme has no entry). `Theme::css` emits a `:root` sheet of the same names for the browser. Nested `var()`, non-color variables, and opacity modifiers on `var()` stay unsupported.
