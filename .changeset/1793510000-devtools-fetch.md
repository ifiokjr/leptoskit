---
"release": minor
---

# Download the DevTools interface for the CLI's version

`leptoskit dev` downloads the DevTools interface the first time it runs, from the GitHub release for exactly the CLI's version, and checks it against a SHA-256 built into the CLI. A mismatch is an error and is never retried silently, and no other version is ever used. `leptoskit devtools fetch` downloads it ahead of time, for CI or for working offline. A CLI built from source has no hash and explains how to serve a local build with `LEPTOSKIT_DEVTOOLS_UI` instead.
