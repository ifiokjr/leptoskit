<h1 align="center">leptoskit_plugin_updater</h1>

<p align="center">
  <strong>leptoskit_plugin_updater</strong> is the leptoskit plugin for installed native application update hooks.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_updater"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_updater"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_updater"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_updater.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_updater"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_updater/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/leptoskit/leptoskit/ci.yml?label=CI"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Hooks for updating the installed native application itself: check, download, install-and-restart, and status. The host supplies a `Backend` that knows the feed, signing policy, and installer. `UnavailableBackend` keeps unsupported targets compiling; `MemoryBackend` is for tests and scaffolding. This is not Wasm/guest over-the-air replacement — that stays in `leptoskit_codepush`. No update server or feed format ships in this crate.

## How it fits

<!-- {=p_updaterSummary} -->

Native hosts construct `Updater` with an explicit `Backend` that knows how to check, download, and install a new native executable. There is no automatic `native.plugins` registration and no built-in update server. The default `UnavailableBackend` keeps unsupported targets compiling. Guest bundle / Wasm over-the-air updates remain `leptoskit_codepush`; this crate is the installed-application updater surface, matching Electron's `autoUpdater` responsibility without shipping a specific feed format.

<!-- {/p_updaterSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_updater::MemoryBackend;
use leptoskit_plugin_updater::Updater;

let mut updater = Updater::new(MemoryBackend::new(
	"1.0.0",
	Some(("1.1.0".into(), Some("bug fixes".into()))),
)?);
updater.check()?;
updater.download()?;
updater.install_and_restart()?;
```

## Learn more

- [Electron → leptoskit map](https://leptoskit.dev/docs/platforms/electron.html)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
- [Repository](https://github.com/leptoskit/leptoskit)
