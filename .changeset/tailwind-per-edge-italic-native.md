---
"leptoskit_native": patch
"leptoskit_gtk": patch
"leptoskit_android": patch
---

# Paint italic text and per-edge borders on Linux and Android

`border-t`, `border-l-2`, and the other edge border utilities now get the same default stroke color as a uniform `border`, and GTK and Android paint each edge instead of leaving those strokes undrawn. The `italic` utility also draws through GTK labels and Android text views, matching the web and the terminal. AppKit, UIKit, and Win32 still leave per-edge borders undrawn and italic upright.
