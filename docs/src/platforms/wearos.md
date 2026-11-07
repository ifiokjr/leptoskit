# Wear OS

<Callout kind="warning" title="Experimental">

Wear OS support is early. The Rust watch API and Kotlin host stubs exist; APK packaging, Play's Wear track upload, and live tile or complication updates do not. Use it to design Wear surfaces, not to ship.

</Callout>

On Wear OS, leptoskit does not pretend a phone activity is a watch. You describe a `WatchApp` with pages, tiles, and complications. The experimental host under `hosts/wearos` names the Wear entry points a future generated app will bind: a watch activity, a `TileService`, and a complication data source.

## What works and what doesn't

| Works                                                                   | Doesn't work yet                                  |
| ----------------------------------------------------------------------- | ------------------------------------------------- |
| `WatchApp` / `WatchPage` / `WatchTile` / `WatchComplication` validation | Packaging an APK or App Bundle                    |
| Recording presentations in `WearOsHost` for tests                       | Installing on an emulator or watch                |
| Tailwind classes and `View` / `Text` / `Button` inside budgets          | Live tile timelines and complication updates      |
| `leptoskit new --platform wearos` and `native prepare wearos`           | Code push, DevTools, and runtime plugins on watch |

## Create a project

```sh
leptoskit new steps --platform wearos
cd steps
leptoskit native prepare wearos
```

The starter depends on `leptoskit_watch` and `leptoskit_wearos`. Guest-shaped content lives in `hosts/wearos/guest.rs`. Kotlin stubs live under `hosts/wearos/android/`.

## Manifest requirements (when packaging lands)

- `android.hardware.type.watch` uses-feature (required for Wear distribution).
- A launcher activity (or tile-only distribution where Play allows it).
- Tile service with `androidx.wear.tiles.action.BIND_TILE_PROVIDER`.
- Complication data source intent filters matching the families you declare.
- Play Console Wear OS form factors for store listing.

Until packaging lands, keep these as notes next to the stubs; the prepare step does not write a Gradle project yet.

## Build

```sh
leptoskit native build wearos
```

Stops with an experimental notice. Prefer unit tests against `WearOsHost` and `WatchApp::validate` while the host is a stub.
