# Wear OS (experimental)

`__APP_NAME__` targets Android Wear OS through leptoskit's watch UI API (`leptoskit::watch`). Package id: `__PACKAGE_ID__`.

This host is a scaffold. The Kotlin stubs under `android/` name the Wear OS entry points (watch activity, tile service, complication data source). They do not yet package an APK or install on a watch.

## Surfaces

Write watch surfaces with `WatchApp`, `WatchPage`, `WatchTile`, and `WatchComplication` — not phone file routes. See the Wear OS platform page in the docs.

## Next steps

```sh
# API and host stubs are ready; packaging is not:
leptoskit native prepare wearos
# leptoskit native build wearos   # errors with an experimental notice
```

Enable the platform in `leptoskit.toml`:

```toml
[app]
platforms = ["wearos"]
default_platform = "wearos"

[native]
package_id = "__PACKAGE_ID__"
targets = ["wearos"]
```
