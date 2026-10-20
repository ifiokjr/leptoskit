# leptoskit_axum

> The Axum server toolkit for leptoskit projects.

Axum is the one supported server. This crate wraps the pieces every deployment needs: static asset serving with pre-compressed gzip and correct wasm MIME types, an SPA fallback to `index.html` for `GET`/`HEAD` requests whose last path segment has no file extension (missing assets stay `404`), health checks at `/_leptoskit/health`, and the code push registry under `/api/codepush`. SSR and leptos server functions mount through `leptos_axum` in the app itself; this crate stays leptos-free to keep the server binary small.
