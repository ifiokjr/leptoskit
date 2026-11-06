---
leptoskit_router: major
---

# leptoskit_router: typed file routes

`leptoskit_router` is the runtime behind leptoskit file routes: each file in `src/routes/` becomes a variant of one generated `Route` enum, so URLs are parsed into typed values and links are written from them, and the two cannot drift apart.

`leptoskit dev`, `leptoskit build`, and `leptoskit routes` generate the enum; the app then holds a `Router` to read the current route, `navigate`, `replace`, go `back`, and build links that render as real `<a href>` links on the web. Each page renders inside the layouts of the folders above it. Natively the history lives in memory; the `web` feature keeps the browser's address bar and back button in step. Routes match from most to least specific, and parameter structs use `#[derive(Route)]`.
