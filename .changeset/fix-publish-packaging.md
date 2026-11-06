---
"release": patch
---

# Restore publishable packaging for the workspace

`setup/harmonyos_plugins` now inherits its plugin dependencies from the workspace, and the publish workflow verifies only the crates crates.io receives. `cargo package --workspace` no longer trips over path-only dependencies in private examples and setup fixtures, so a release can reach the registry again.
