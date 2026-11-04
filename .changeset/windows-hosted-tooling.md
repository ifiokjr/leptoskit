---
"release": patch
---

# Compile hosted tooling cleanly on Windows

Keep Unix filesystem sampling and its `rustix` dependency behind Unix target guards. Scope directory-permission bookkeeping and managed source-checkout completion to the platforms that use them, so Windows builds pass strict linting while retaining the existing platform behavior.
