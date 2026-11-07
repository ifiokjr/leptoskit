# watchOS

<Callout kind="warning" title="Experimental">

watchOS support is early. The Rust watch API and Swift host stubs exist; Xcode packaging, simulator install, and live complication or WidgetKit updates do not. Use it to design watch surfaces, not to ship.

</Callout>

On watchOS, leptoskit models pages, Smart Stack glances, and complications as first-class surfaces. A phone UIKit tree is not reused. The experimental host under `hosts/watchos` names the WatchKit app, WidgetKit glance, and complication entry points a future generated app will bind.

## What works and what doesn't

| Works                                                                     | Doesn't work yet                                  |
| ------------------------------------------------------------------------- | ------------------------------------------------- |
| `WatchApp` / `WatchPage` / `WatchGlance` / `WatchComplication` validation | Xcode project generation and `.app` packaging     |
| Recording presentations in `WatchOsHost` for tests                        | Simulator or device install                       |
| Tailwind classes and `View` / `Text` / `Button` inside budgets            | ClockKit / WidgetKit live timelines               |
| `leptoskit new --platform watchos` and `native prepare watchos`           | Code push, DevTools, and runtime plugins on watch |

## Create a project

```sh
leptoskit new steps --platform watchos
cd steps
leptoskit native prepare watchos
```

The starter depends on `leptoskit_watch` and `leptoskit_watchos`. Guest-shaped content lives in `hosts/watchos/guest.rs`. Swift stubs live under `hosts/watchos/swift/`.

## Entitlements (when packaging lands)

- Watch App target, optionally paired with an iOS companion (or watch-only where App Store rules allow).
- Widget Extension for Smart Stack glances.
- Complication push when you update complications from the network.
- App Groups when the watch and phone share a store.
- HealthKit or other capabilities only when a surface needs them.

Until packaging lands, keep these as notes next to the stubs; the prepare step does not write an Xcode project yet.

## Build

```sh
leptoskit native build watchos
```

Stops with an experimental notice. Prefer unit tests against `WatchOsHost` and `WatchApp::validate` while the host is a stub.
