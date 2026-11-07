# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-07)

### Features

#### leptoskit_plugin_device_policy: Android device admin

_Owner:_ [@ifiokjr](https://github.com/ifiokjr)

`leptoskit_plugin_device_policy` is the leptoskit plugin for explicit Android Device Admin / Device Policy Manager capabilities such as lock, password rules, camera disable, and confirmed wipe.

Add `leptoskit_plugin_device_policy` as a direct dependency and select the `device-policy` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::DevicePolicy` rather than resolving the plugin on every call. Only Android implements Device Admin / Device Policy Manager capabilities; every other platform returns `Unavailable`. Status never prompts. Privileged actions require an active device admin the user enabled, and wipe requires an explicit confirmation token.
