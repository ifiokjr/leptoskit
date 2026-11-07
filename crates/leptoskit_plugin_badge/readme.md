<h1 align="center">leptoskit_plugin_badge</h1>

<p align="center">
  <strong>leptoskit_plugin_badge</strong> is the leptoskit plugin for app icon badge counts.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_badge"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_badge"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_badge"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_badge.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_badge"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_badge/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Sets, reads, and clears a badge count up to 999,999. iOS and macOS use the system badge APIs. Android stores the requested count and clears the app's notifications when the count is zero; launcher vendors that key badges off notifications still decide visibility. Linux exposes a direct Unity/KDE `LauncherEntry` D-Bus client for custom hosts. Generated Windows/Linux runners do not register runtime plugins yet. The browser Badging API is not wired in this port.

Enable `guest` for native bridge calls. Select `badge` in `native.plugins`. No permissions are declared by this crate; showing a badge may still require notification authorization on some platforms.

A successful set only means the OS accepted the request. Focus modes and user settings can hide the badge.

## How it fits

<!-- {=p_badgeSummary} -->

Add `leptoskit_plugin_badge` as a direct dependency and select the `badge` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Badge` rather than resolving the plugin on every call. Setting a count asks the OS to show an app-icon badge; it never proves the badge is visible under the user's focus or notification settings.

<!-- {/p_badgeSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_badge::guest::Badge;

let badge = Badge::new()?;
badge.set(3).await?;
assert_eq!(badge.get().await?, 3);
badge.clear().await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_badge)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
