# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-07)

### Breaking changes

#### leptoskit_plugin_home_widgets: home-screen and desktop widgets

_Owner:_ [@ifiokjr](https://github.com/ifiokjr)

`leptoskit_plugin_home_widgets` is the leptoskit plugin for publishing bounded widget snapshots to iOS and macOS WidgetKit and Android App Widgets, then asking the OS to reload them.

Add `leptoskit_plugin_home_widgets` as a direct dependency and select the `home-widgets` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Guests enable the `guest` feature and reuse `guest::HomeWidgets`. The plugin does not draw widget UI or scaffold extension targets; apps still own their WidgetKit extensions and App Widget providers.
