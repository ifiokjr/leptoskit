<h1 align="center">leptoskit_plugin_app_shortcuts</h1>

<p align="center">
  <strong>leptoskit_plugin_app_shortcuts</strong> is the leptoskit plugin for bounded home-screen shortcuts, dock items, and jump-list style launcher actions.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_app_shortcuts"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_app_shortcuts"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_app_shortcuts"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_app_shortcuts.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_app_shortcuts"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_app_shortcuts/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Publishes up to eight static shortcuts with portable ids, titles, and optional subtitles. iOS uses home-screen quick actions, Android uses `ShortcutManager` dynamic shortcuts, and macOS installs a dock menu. Windows jump-list publishing and Linux desktop-action publishing are not registered by generated hosts yet; the crate's in-memory `Registry` is for custom hosts and tests. The browser is unavailable: PWA shortcuts belong in the web app manifesto.

Enable `guest` for native bridge calls. Select `app-shortcuts` in `native.plugins`. No permissions or entitlements are added. The host must call `noteLaunch` when the OS activates a shortcut so `consume_launch` can return that id once.

This port does not add icons, deep-link handlers, Siri App Intents, or pinned/manifest shortcuts beyond the dynamic list.

## How it fits

<!-- {=p_app_shortcutsSummary} -->

Add `leptoskit_plugin_app_shortcuts` as a direct dependency and select the `app-shortcuts` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::AppShortcuts` rather than resolving the plugin on every call. The plugin publishes a bounded static shortcut list to the OS launcher, dock, or jump list; it never invents icons or deep-link handlers.

<!-- {/p_app_shortcutsSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_app_shortcuts::{Shortcut, ShortcutList, guest::AppShortcuts};

let shortcuts = AppShortcuts::new()?;
let list = ShortcutList::new([
    Shortcut::new("compose", "Compose", "New note")?,
    Shortcut::new("search", "Search", "")?,
])?;
shortcuts.set(&list).await?;
if let Some(id) = shortcuts.consume_launch().await? {
    // route to the matching screen
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_app_shortcuts)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
