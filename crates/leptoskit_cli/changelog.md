# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1](https://github.com/ifiokjr/leptoskit/releases/tag/v0.1.1) (2026-11-01)

### Features

#### Add foreground native services and readable Android chrome

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #62](https://github.com/ifiokjr/leptoskit/pull/62)

Add independent local device-owner authentication, foreground location and immediate local notification crates. Selected services register through Swift/Kotlin metadata. Permission prompts remain explicit, purpose strings remain app-owned, and cancellation/close release native work. Browser adapters expose supported location and notification operations.

Authentication does not identify an account or authorize a server request. Notification scheduling, push registration and action routing, background location, and Windows/Linux local authentication remain unimplemented. Generated Android applications use an app-owned light theme with dark system-bar icons.

### Fixes

- **Keep native Wasm guests optimized in debug apps.** Compile interpreted guests with the configured release profile while retaining native host debugging and the existing bundle size limit. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #230](https://github.com/ifiokjr/leptoskit/pull/230)
- **Expose labelled iOS text as an accessibility value.** Keep visible UILabel content readable alongside a custom accessibility label. Refresh and clear the value when props change, explicitly restore ordinary text labels after removing a custom label, and isolate UIKit style mutation to the main actor. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #232](https://github.com/ifiokjr/leptoskit/pull/232)
- **Remove Android Button's native padding before applying shared layout frames so compact buttons paint their labels.** _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #172](https://github.com/ifiokjr/leptoskit/pull/172)
- **Allow one runtime package to install its Rust application service alongside a distinct native plugin on macOS, iOS, and Android.** Generated hosts retain both registrations and their native sources, imports, and context while preserving the one-service limit and duplicate capability rejection. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #181](https://github.com/ifiokjr/leptoskit/pull/181)
