---
"release": patch
---

# Paint the whole screen with the root's background on iOS and Android

A dark screen in a generated iOS or Android app stopped at the safe area: the status bar, the home indicator, and Android's navigation bar stayed white around it. The generated apps now paint those strips with the root view's opaque background, the way a browser paints the page's background across the whole window, and switch the system bar icons to light on a dark background and dark on a light one. A root without an opaque background keeps the system's background, as before.

The code-push update bar keeps its light background whatever the guest paints.
