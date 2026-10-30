# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1](https://github.com/ifiokjr/leptoskit/releases/tag/v0.1.1) (2026-10-30)

### Features

#### Add owned desktop shortcuts and window placement

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #62](https://github.com/ifiokjr/leptoskit/pull/62)

Add independent global-shortcut and window-state crates. Shortcuts require explicit event-loop ownership, bound registrations and event queues, and release native registrations on teardown. Window placement validates portable geometry; macOS capture/restore operates on an explicitly owned window, with persistence controlled by the application.

Wayland shortcuts and Windows/Linux native window capture/restore remain unimplemented. Extend the generated Android service fixture with logging/crypto worker assertions, scoped service checks and system-bar verification. The catalog records all eight service crates and the crypto extension with operation-level platform and registration limits.
