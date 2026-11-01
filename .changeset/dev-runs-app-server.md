---
"release": patch
---

# Run a client-rendered app's own server in `leptoskit dev`

A client-rendered app with a server package, such as one with an API or sign-in, now runs that server under `leptoskit dev` and `leptoskit serve`, and the server serves the built site. Before, both served `dist/` from leptoskit's static server, so the app's API calls failed with 405.
