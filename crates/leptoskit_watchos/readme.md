<h1 align="center">leptoskit_watchos</h1>

<p align="center">
  <strong>leptoskit_watchos</strong> is the experimental Apple watchOS backend for leptoskit watch surfaces: pages, WidgetKit glances, and ClockKit complications described with [`leptoskit_watch`].
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_watchos"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_watchos"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_watchos.svg"></a>
  <a href="https://docs.rs/leptoskit_watchos"><img alt="docs.rs" src="https://docs.rs/leptoskit_watchos/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Status

**Experimental.** The Rust API validates and records presentations. The Swift stubs under `swift/` name the watchOS entry points (WatchKit app, WidgetKit glance, ClockKit / WidgetKit complications) but do not yet drive a simulator. Packaging through `leptoskit native build watchos` is not shipping yet.

## Entitlements notes

watchOS apps need a Watch App target paired with an iOS companion (or a watch-only app where App Store rules allow it), the HealthKit / Complication push entitlements your surfaces use, and App Groups when sharing data with the phone. See the public watchOS platform page in the docs.
