<h1 align="center">leptoskit_wearos</h1>

<p align="center">
  <strong>leptoskit_wearos</strong> is the experimental Android Wear OS backend for leptoskit watch surfaces: pages, tiles, and complications described with [`leptoskit_watch`].
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_wearos"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_wearos"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_wearos.svg"></a>
  <a href="https://docs.rs/leptoskit_wearos"><img alt="docs.rs" src="https://docs.rs/leptoskit_wearos/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Status

**Experimental.** The Rust API validates and records presentations. The Kotlin host stubs under `android/` name the Wear OS entry points (`TileService`, complication data sources, watch activity) but do not yet drive a device or emulator. Packaging through `leptoskit native build wearos` is not shipping yet.

## Manifest notes

Wear OS apps need a watch activity (or tile-only distribution), the `android.hardware.type.watch` feature, and Play Console's Wear OS track. Tiles use `androidx.wear.tiles`; complications use `androidx.wear.watchface.complications`. See the public Wear OS platform page in the docs.
