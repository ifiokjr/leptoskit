---
leptoskit_bridge_ffi: patch
leptoskit_cli: patch
---

# Expose labelled iOS text as an accessibility value

Keep visible UILabel content readable alongside a custom accessibility label. Refresh and clear the value when props change, explicitly restore ordinary text labels after removing a custom label, and isolate UIKit style mutation to the main actor.
