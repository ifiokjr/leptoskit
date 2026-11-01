# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1](https://github.com/ifiokjr/leptoskit/releases/tag/v0.1.1) (2026-11-01)

### Features

#### Add foreground native services and readable Android chrome

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #62](https://github.com/ifiokjr/leptoskit/pull/62)

Add independent local device-owner authentication, foreground location and immediate local notification crates. Selected services register through Swift/Kotlin metadata. Permission prompts remain explicit, purpose strings remain app-owned, and cancellation/close release native work. Browser adapters expose supported location and notification operations.

Authentication does not identify an account or authorize a server request. Notification scheduling, push registration and action routing, background location, and Windows/Linux local authentication remain unimplemented. Generated Android applications use an app-owned light theme with dark system-bar icons.
