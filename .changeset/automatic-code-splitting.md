---
"release": minor
---

# Split lazy web code by default

Lazy functions, routes, and islands now build into on-demand wasm chunks without setting `bundle.split`. Clients without lazy code skip splitting and keep their existing binding cache. Explicit `split = false` remains available and rejects lazy imports before writing an unusable client. Environment rustflags only require relocations when the client actually contains lazy code.

The native CodePush investigation includes a reproducible wasmi probe and a proposed shared API; native chunk loading is not yet a supported application feature.
