---
"release": patch
---

# Preserve generated host compatibility with widget wire names

Generated UIKit and Android hosts accept the canonical widget names serialized by the Rust renderer, while retaining legacy names. This restores initial interface rendering after the shared component serialization update.
