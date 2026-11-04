---
"release": major
---

# Start the first release at 0.2.0

This is leptoskit's first public release, and every crate in it is `0.2.0`. The workspace, its internal crate dependencies, and the example apps that pin them all start from `0.1.0`, and a breaking (`major`) change before 1.0 raises the minor version, so the release lands on `0.2.0`. Several of the crate names already have a `0.1.0` on crates.io, which never accepts the same version twice, so the first release has to sit above it.

Apps that `leptoskit new` creates pin the exact version of the CLI that made them, `=0.2.0`.
