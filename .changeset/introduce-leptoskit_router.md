---
leptoskit_router: breaking
---

# leptoskit_router: typed file routes

`leptoskit_router` is the runtime behind leptoskit file routes: `src/routes/` becomes one `Route` enum that parses URLs and writes hrefs.

The CLI's `leptoskit routes` runs the codegen; `leptoskit_plugin_router` runs it before cargo; this crate is what app code imports. A route file is the only place a URL shape is declared, so links and parsing cannot drift.
