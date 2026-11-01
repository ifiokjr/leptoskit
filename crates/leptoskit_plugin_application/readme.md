<h1 align="center">leptoskit_plugin_application</h1>

<p align="center">
  <strong>leptoskit_plugin_application</strong> is the leptoskit plugin for installed host application identity: ID, localized display name, release version, and build number.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_application"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_application"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_application.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_application"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_application/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Installed host application identity: ID, localized display name, release version, and build number. Apple implementations read `Bundle.main`; Android reads the running application's `PackageManager` record. This is the host's identity even when the interpreted guest is updated independently.

Add this crate as a direct dependency and `"application"` to `native.plugins` for automatic Apple/Android registration. On Rust desktop hosts, construct `Application::new(&Snapshot { ... })` with the host's packaging metadata. This validates and caches the encoded reply. The library does not use its own Cargo package name or version as the app's identity. Apple desktop bundles use the same Swift implementation as iOS.

Browsers expose no equivalent installed-native-app identity. Browser applications can provide their own explicitly configured `Snapshot`; document titles, origins, and this library's version are not substituted for native package metadata. Missing fields are `None`. No device IDs, vendor IDs, install referrers, release channel guesses, installation dates, or store environment APIs are included.

The initial port covers the four identity constants from [Expo Application](https://docs.expo.dev/versions/latest/sdk/application/).

## How it fits

Add `leptoskit_plugin_application` as a direct dependency and select the `application` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Application` rather than resolving the plugin on every call. Apple reads `Bundle.main`, Android reads the running package record, and a Rust desktop host supplies its own packaging metadata; browsers have no equivalent installed-app identity.

## Use it

```rust,ignore
let app = leptoskit_plugin_application::guest::Application::new()?;
let identity = app.snapshot().await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_application)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
