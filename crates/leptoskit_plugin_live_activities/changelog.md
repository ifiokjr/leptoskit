# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_plugin_live_activities: Live Activities

`leptoskit_plugin_live_activities` is the leptoskit plugin for starting, updating, and ending iOS Live Activities (including Dynamic Island presentations) with optional ActivityKit push-token hooks.

Add `leptoskit_plugin_live_activities` as a direct dependency and select the `live-activities` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::LiveActivities` rather than resolving the plugin on every call. Only iOS implements ActivityKit; other platforms return `Unavailable`. Starting an activity does not prove Lock Screen or Dynamic Island UI appeared: the app must ship a Widget Extension that renders `LeptosKitLiveActivityAttributes`.
