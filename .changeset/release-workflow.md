---
"release": minor
---

# A release workflow that publishes on a version tag

`leptoskit new` now writes `.github/workflows/release.yml`, and `leptoskit publish workflow` adds it to an existing project. On a `v<version>` tag it checks the tag against `app.version`, installs the matching CLI, sets up only what the configured stores need, and runs `leptoskit publish` with the credentials from repository secrets. A tag only uploads; running the workflow by hand with "submit" checked submits. Without a `[publish]` table it does nothing.
