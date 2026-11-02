---
leptoskit_router_macros: breaking
---

# leptoskit_router_macros: the Route derive

`leptoskit_router_macros` provides the `#[derive(Route)]` macro that reads a route struct's fields from the URL by name.

It is an implementation detail of `leptoskit_router`, which re-exports the derive. `Option<T>` may be absent, `Vec<T>` takes every value, and other types are required unless the field is `#[route(default)]`.
