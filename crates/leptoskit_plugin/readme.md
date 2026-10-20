<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_plugin

> The plugin system shared by leptoskit builds and leptos plugins.

Two kinds of plugins ride the same `leptoskit.toml` declaration:

- **leptoskit plugins** implement the `LeptoskitPlugin` trait — `before_build`, `after_build`, `before_serve` hooks that add cargo features, environment variables, asset directories, and post-build reporting. Built-ins ship with the crate (`env_manifest`, `size_report`); local plugin crates declare a `path` and compile into the tooling.
- **leptos plugins** declare `leptos = true` and are forwarded to the app build as cargo features, which is how the leptos plugin ecosystem activates. Their options table survives into the build untouched.

```toml
[[plugins]]
name = "size_report"

[[plugins]]
name = "leptos_meta"
leptos = true
```
