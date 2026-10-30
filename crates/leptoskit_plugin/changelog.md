# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1](https://github.com/ifiokjr/leptoskit/releases/tag/v0.1.1) (2026-10-30)

### Features

#### Plugin hooks for codegen, commands, native needs, serving

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #194](https://github.com/ifiokjr/leptoskit/pull/194)

`LeptoskitPlugin` grows defaulted hooks, so existing plugins compile unchanged: `generate` runs before cargo on every `leptoskit build` and dev-server rebuild and records the files it wrote so the dev watcher ignores them, `run_command` answers a plugin's own `plugin:<namespace> <command>` commands, `native_integration` declares the Android permissions, Maven dependencies, and Apple usage descriptions a generated host needs, and `metadata` declares the plugin's CLI namespace and supported platforms — a build for an unsupported platform now fails naming the plugin.

`before_serve`, which existed but was never called, now runs before the `dev` and `serve` servers start.
