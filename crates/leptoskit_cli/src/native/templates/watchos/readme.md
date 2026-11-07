# watchOS (experimental)

`__APP_NAME__` targets Apple watchOS through leptoskit's watch UI API (`leptoskit::watch`). Bundle id base: `__PACKAGE_ID__`.

This host is a scaffold. The Swift stubs under `swift/` name the watchOS entry points (WatchKit app, WidgetKit glance, complications). They do not yet produce an app for the simulator.

## Surfaces

Write watch surfaces with `WatchApp`, `WatchPage`, `WatchGlance`, and `WatchComplication` — not phone file routes. See the watchOS platform page in the docs.

## Next steps

```sh
# API and host stubs are ready; packaging is not:
leptoskit native prepare watchos
# leptoskit native build watchos   # errors with an experimental notice
```

Enable the platform in `leptoskit.toml`:

```toml
[app]
platforms = ["watchos"]
default_platform = "watchos"

[native]
package_id = "__PACKAGE_ID__"
targets = ["watchos"]
```
