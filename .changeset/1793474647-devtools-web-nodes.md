---
"release": minor
---

# Mark web nodes for DevTools

With the `devtools` feature, every rendered web node carries its path in `data-lk-node`, so picking a DOM node maps back to the element. Without the feature the attribute is not emitted.
