---
leptoskit_axum: breaking
---

# leptoskit_axum: the server toolkit

`leptoskit_axum` is the Axum server toolkit every leptoskit deployment shares: static files with precompressed copies, SPA fallback, health checks, and code push routes.

The generated server mounts it beside `leptos_axum`; apps that serve their own assets use the same functions. It stays leptos-free so the server binary only pays for what it uses.
