---
"release": major
---

# Start the first release at 0.1.0

Every crate now starts from the workspace version `0.0.0`, including the versions the workspace records for its internal crates and the scaffolded apps that pin them. monochange therefore treats a breaking change as a pre-1.0 minor bump, so the first release is `0.1.0` rather than `0.1.1`.

The release workflow writes its commit and opens its pull request through the monochange GitHub App, which creates a verified bot commit. The checks that prepare that release also keep the notification plugin, dev-server test, deploy snapshots, and native benchmark policy in step with the new version.
