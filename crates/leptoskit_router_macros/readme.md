<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_router_macros

> The `#[derive(Route)]` macro for leptoskit file routes.

An implementation detail of [`leptoskit_router`](../leptoskit_router), which re-exports the derive. It implements `RouteParams` for a route struct: each field is read from the URL by name, and its type decides how — `Option<T>` may be absent, `Vec<T>` takes every value, and any other `T` is required unless the field is `#[route(default)]`.
