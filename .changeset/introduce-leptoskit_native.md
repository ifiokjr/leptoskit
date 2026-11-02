---
leptoskit_native: breaking
---

# leptoskit_native: components, Tailwind, and the renderer

`leptoskit_native` is the native side of the shared tree: builders for every component, the Tailwind compiler, the retained element tree, and the diff that turns a commit into a mutation batch.

Every native backend implements `NativeBackend` and consumes the mutations this crate produces; the web build uses its optional `web` feature to render the same builders through leptos. App code normally reaches it through `leptoskit::prelude`, not as a direct dependency, so one screen works on every target.
