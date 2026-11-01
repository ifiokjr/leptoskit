---
"release": minor
---

# `leptoskit publish macos`: notarized downloads

`leptoskit publish macos` signs the macOS app with a Developer ID (`LEPTOSKIT_PUBLISH_MACOS_IDENTITY`), with the hardened runtime and a secure timestamp, signing its Safari extension first. It notarizes a `.dmg` of it with the App Store Connect API key, staples Apple's ticket to both, and writes the `.dmg` and a `.zip` to `.leptoskit/publish/macos/`, which Gatekeeper opens on any Mac.

Configure it with `[publish.macos]`. With `github_release = "owner/name"`, `--submit` attaches both files to the GitHub release `v<app.version>`.
