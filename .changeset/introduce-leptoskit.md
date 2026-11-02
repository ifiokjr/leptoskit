---
leptoskit: breaking
---

# leptoskit: the one dependency

`leptoskit` is the crate an app depends on: it re-exports the shared components, Tailwind styles, renderers, and runtime behind one feature per optional part.

Start here. A web build enables `web` and renders through leptos; a native build enables a platform backend and commits the same tree as real widgets. Features add routing, queries, forms, localization, database, auth, extensions, and DevTools only when the app asks for them, so the meta crate stays a front door rather than a bundle of everything.
