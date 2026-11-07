<h1 align="center">leptoskit_plugin_home_widgets</h1>

<p align="center">
  <strong>leptoskit_plugin_home_widgets</strong> is the leptoskit plugin for home-screen and desktop widget snapshots on iOS WidgetKit, Android App Widgets, and macOS WidgetKit.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_home_widgets"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_home_widgets"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_home_widgets"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_home_widgets.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_home_widgets"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_home_widgets/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Publish a bounded text snapshot from the Wasm guest, store it where a native widget extension can read it, and ask the OS to reload WidgetKit timelines or Android App Widgets. A successful call means the host accepted the snapshot or reload request. It does not create, pin, or draw a widget, and it does not prove that a home-screen instance exists.

| Platform        | Implementation                                  | Scope                                                                                     |
| --------------- | ----------------------------------------------- | ----------------------------------------------------------------------------------------- |
| iOS / macOS     | Swift `UserDefaults` + `WidgetCenter`           | Capability, set/get/clear snapshot, reload timelines; installed state is always `Unknown` |
| Android         | Kotlin `SharedPreferences` + `AppWidgetManager` | The above, plus best-effort installed queries against the app's own App Widget providers  |
| Browser         | Unavailable                                     | No home-screen widget backend                                                             |
| Windows / Linux | Unavailable                                     | No widget backend yet                                                                     |

The app still owns the WidgetKit extension target or `AppWidgetProvider`. On Apple, pass an App Group identifier when constructing `HomeWidgetsPlugin` so the extension and the host share the same suite; without it, snapshots land in standard `UserDefaults` and capability reports `shared_store = false`. On Android, providers are discovered from the app package's `APPWIDGET_UPDATE` receivers; kind-to-provider matching is best-effort by class name.

Interactive widget controls, timeline entries with multiple dates, configuration intents, glance/complication surfaces, and scaffolded extension templates are not implemented.

## Binary protocol

All integers are little-endian `u32`. Strings are `u32` byte lengths followed by strict UTF-8. Every decoder rejects trailing bytes, truncated fields, invalid UTF-8 and values outside their domain before calling a platform API.

| ID | Method     | Request                                | Response         |
| -- | ---------- | -------------------------------------- | ---------------- |
| 0  | Capability | Empty                                  | Capability flags |
| 1  | Set        | Kind, family, title, body, URL strings | Empty            |
| 2  | Get        | Kind string                            | Snapshot         |
| 3  | Clear      | Kind string                            | Empty            |
| 4  | Reload     | Empty (all) or kind string             | Empty            |
| 5  | Installed  | Kind string                            | Installed `u32`  |

Capability bits: supported=1, can_reload=2, can_query_installed=4, shared_store=8. Family is System=0, Small=1, Medium=2, Large=3 or ExtraLarge=4. Installed is Unknown=0, None=1 or Some=2. Kinds are 1–64 ASCII letters, digits, `.`, `_` or `-`. Titles have at most 256 UTF-8 bytes, bodies 1024, URLs 2048. Titles must contain non-whitespace text. Control characters are rejected except line feeds and tabs in bodies. Empty URLs mean no deep link; non-empty URLs must include a scheme separator.

## How it fits

<!-- {=p_home_widgetsSummary} -->

Add `leptoskit_plugin_home_widgets` as a direct dependency and select the `home-widgets` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::HomeWidgets` rather than resolving the plugin on every call. The plugin publishes bounded snapshot text for WidgetKit and Android App Widgets and asks the OS to reload them; it does not draw the widget UI or scaffold the extension target.

<!-- {/p_home_widgetsSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_home_widgets::{Family, Kind, Snapshot};
use leptoskit_plugin_home_widgets::guest::HomeWidgets;

let widgets = HomeWidgets::new()?;
let capability = widgets.capability().await?;
// Inspect capability.supported / shared_store before relying on reload.
let snapshot = Snapshot::new(
	Kind::new("status")?,
	Family::Medium,
	"Inbox",
	"3 unread",
	"myapp://inbox",
)?;
widgets.set(&snapshot).await?;
widgets.reload(Some(snapshot.kind())).await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_home_widgets)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
