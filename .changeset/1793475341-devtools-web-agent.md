---
"release": minor
---

# Add the web DevTools agent

With the `web` feature, a page reports its `data-lk-node` tree to the hub, sets `window.__LEPTOSKIT_DEVTOOLS_SESSION__`, polls commands, and draws highlight rectangles. The hub URL is compiled in by `leptoskit dev`.
