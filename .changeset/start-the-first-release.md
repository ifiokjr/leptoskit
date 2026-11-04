---
"release": major
---

# Start the first release at 0.2.0

Every crate inherits the workspace version `0.1.0`, as do the versions the workspace records for its internal crates and the scaffolded apps that pin them. monochange treats a breaking change before 1.0 as a minor bump, so the first release is `0.2.0`. Several crate names already carry a `0.1.0` on crates.io, and crates.io never accepts a version twice, so the first release has to sit above it.

The release workflow writes its commit and opens its pull request through the monochange GitHub App, which creates a verified bot commit. The checks that prepare that release also keep the notification plugin, dev-server test, deploy snapshots, and native benchmark policy in step with the new version.
