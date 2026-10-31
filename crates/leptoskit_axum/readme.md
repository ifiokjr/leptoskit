<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_axum

> The Axum server toolkit for leptoskit projects.

Axum is the one supported server. This crate wraps the pieces every deployment needs: static asset serving with pre-compressed brotli and gzip and correct wasm MIME types, an SPA fallback to `index.html` for `GET`/`HEAD` requests whose last path segment has no file extension (missing assets stay `404`), health checks at `/_leptoskit/health`, and the code push registry under `/api/codepush`. SSR and leptos server functions mount through `leptos_axum` in the app itself; this crate stays leptos-free to keep the server binary small. A server-rendered app mounts `static_files` behind its routes to serve the built site the same way, merges `Readiness::routes` for `/healthz` and `/readyz`, and serves with `serve_listener_with` and a `GracefulShutdown`, which fails readiness on `SIGTERM` before it stops accepting and finishes the requests in flight.
