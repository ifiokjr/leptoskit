<h1 align="center">leptoskit_plugin_power_monitor</h1>

<p align="center">
  <strong>leptoskit_plugin_power_monitor</strong> is the leptoskit plugin for desktop power and idle observation.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_power_monitor"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_power_monitor"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_power_monitor"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_power_monitor.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_power_monitor"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_power_monitor/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/leptoskit/leptoskit/ci.yml?label=CI"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Desktop power and session lifecycle observation for hosts that already own an event loop. The host pushes suspend, resume, lock, unlock, AC/battery, shutdown, and idle/active events into `PowerMonitor`. Guests can drain those events and read a snapshot through the bridge when the host registers the plugin. System idle-time queries work on Windows; other platforms return `Unavailable` until a native adapter lands. Battery charge percentage stays in `leptoskit_plugin_battery`, and keeping the display awake stays in `leptoskit_plugin_keep_awake`.

## How it fits

<!-- {=p_power_monitorSummary} -->

Native hosts construct `PowerMonitor` and may expose it through the bridge. There is no automatic `native.plugins` registration. The host pushes suspend/resume and related lifecycle events from its event loop; idle-time queries work on Windows today and return `Unavailable` elsewhere until a platform adapter lands. Mobile and browsers return `Unavailable` for idle queries; the queue and snapshot codecs still compile everywhere.

<!-- {/p_power_monitorSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_power_monitor::Event;
use leptoskit_plugin_power_monitor::PowerMonitor;

let mut monitor = PowerMonitor::new();
monitor.push(Event::OnBattery);
let snapshot = monitor.snapshot();
```

## Learn more

- [Electron → leptoskit map](https://leptoskit.dev/docs/platforms/electron.html)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
- [Repository](https://github.com/leptoskit/leptoskit)
