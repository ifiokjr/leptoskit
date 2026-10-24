---
"release": patch
---

# Reject links to local addresses in docs

A built page that links to `http://localhost:3000`, `http://127.0.0.1:3000`, or another loopback address sends every reader to their own machine, where nothing is listening. GitHub-flavoured autolinks made this easy to do by accident: a bare dev-server URL in prose, or one wrapped in angle brackets, became a link. `Site::build` now reports these links with their file, line, and column, and asks for inline code instead. Addresses inside code spans and code blocks are never links, so they are unaffected.
