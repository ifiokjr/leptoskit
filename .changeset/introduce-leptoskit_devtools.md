---
leptoskit_devtools: breaking
---

# leptoskit_devtools: the DevTools agents

`leptoskit_devtools` contains the agents that report a running app to the local DevTools hub: the web page, the native host, and the Wasm guest.

Under `leptoskit dev`, the CLI starts the hub and these agents open sessions, mirror trees, and carry commands back. Hooks expose the route, named state, forms, and query caches that only the app knows.
