---
"release": minor
---

# Build and run leptos SSR and islands apps

`app.render` picks how a web build renders: `"csr"` (the default), `"ssr"`, or `"islands"`. It replaces the `app.ssr` and `app.hydration` flags, which nothing read. In the server-rendering modes, `leptoskit build` compiles the server package with its `ssr` feature and the wasm client with its `hydrate` feature, adds `leptos/islands` to both in islands mode, and stops with a message naming the package and the fix when a feature or the leptos dependency is missing.

One package can now be both the server binary and the `cdylib` client: the client build compiles only the library, and the app package wins client detection when it has a `cdylib` target.

`leptoskit dev` and `leptoskit serve` run the app's own server binary in the server-rendering modes, with the `LEPTOS_*` environment pointing at the built site, and `dev` restarts it after each rebuild. `leptoskit_axum::static_files` serves the built site behind an app's own routes.

A new `islands` example shows a complete islands app built by leptoskit.
