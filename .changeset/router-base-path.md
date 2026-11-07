---
"release": minor
---

# File routes under a sub-path

`Router::from_browser` reads the page's `<base href>`. An app served under `/app/` declares `<base href="/app/" />`: its routes match the path after it, and every link and history entry the router writes starts with it. Without a `<base>`, the app is at the root of its site, as before.
