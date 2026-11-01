---
"release": patch
---

# Leave empty bindings out of an element's debug output

An `Element`'s `Debug` output listed `bindings: Bindings { text: None, background: None }` on every node, including the ones that bind nothing, so a snapshot or digest of an unchanged tree changed when bindings were added. Bindings now appear only on nodes that have one, as a reactive region already did.
