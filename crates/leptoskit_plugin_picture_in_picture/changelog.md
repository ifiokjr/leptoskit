# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-07)

### Features

#### leptoskit_plugin_picture_in_picture: mobile picture-in-picture and overlays

`leptoskit_plugin_picture_in_picture` is the leptoskit plugin for cross-mobile picture-in-picture and Android system overlay windows.

Add `leptoskit_plugin_picture_in_picture` as a direct dependency and select the `picture-in-picture` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::PictureInPicture` rather than resolving the plugin on every call. Android can enter activity picture-in-picture and, with `SYSTEM_ALERT_WINDOW`, hold a system overlay window. iOS reports whether system picture-in-picture is supported but does not yet enter it without an attached video player, and has no draw-over-apps overlay. Desktop and the browser return `Unavailable`.
