---
leptoskit_extension: breaking
---

# leptoskit_extension: browser extensions

`leptoskit_extension` is the runtime and typed WebExtensions bindings behind leptoskit browser extensions for Chromium, Firefox, and Safari.

Apps enable the `extension` feature, name entry points with `entry!`, and build with `--platform extension`. APIs return `Unavailable` instead of throwing, and listeners are removed when their handle drops.
