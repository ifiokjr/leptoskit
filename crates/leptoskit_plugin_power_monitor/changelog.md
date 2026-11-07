# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-07)

### Features

#### leptoskit_plugin_power_monitor: desktop power events

_Owner:_ [@ifiokjr](https://github.com/ifiokjr)

First-wave Electron `powerMonitor` surface: host-pushed suspend/resume/lock/AC events, bounded event drain, snapshots, and Windows idle-time queries. Other platforms compile with `Unavailable` idle adapters until native observers land.
