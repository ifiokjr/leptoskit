---
"release": patch
---

# Paint the whole screen and draw every shape and label

A dark screen in a generated iOS or Android app stopped at the safe area: the status bar, the home indicator, and Android's navigation bar stayed white around it. The generated apps now paint those strips with the root view's opaque background, the way a browser paints the page's background across the whole window, and switch the system bar icons to light on a dark background and dark on a light one. A root without an opaque background keeps the system's background, as before.

The code-push update bar keeps its light background whatever the guest paints.

Pill shapes drew nothing in generated iOS apps. `rounded-full` asks for a 9999-point radius, and a clipping layer whose radius exceeds half its size draws nothing, so pill buttons, badges, and progress tracks vanished. The radius is now scaled to fit each frame, as browsers and the UIKit backend crate already do.

Generated Android apps truncated each widget's frame to whole pixels, dropping up to a pixel. With measured text, a label sized exactly to its words wrapped its last word out of view. Frames now snap their edges outward instead.
