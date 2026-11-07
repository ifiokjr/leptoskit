---
"release": patch
---

# The live example on leptoskit.dev answers sooner

The landing page's live example loaded the whole hosted app's Wasm client, management console included, before its button worked. The console now loads as a lazy chunk of its own, the client builds with `optimize = "smallest"`, and the page starts the client, its glue, and the Rive runtime downloading together instead of one after another. The example's module drops from 370 KiB to 188 KiB gzip-compressed, and on a phone emulated with Lighthouse's slow 4G and 4x CPU slowdown its first press updates the count about half as long after the example scrolls into view. The hosted console loads the same module and then its chunk, and says the app could not load if the chunk never arrives.
