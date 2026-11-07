<h1 align="center">leptoskit_plugin_nfc</h1>

<p align="center">
  <strong>leptoskit_plugin_nfc</strong> is the leptoskit plugin for bounded one-shot NDEF text and URI reads.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_nfc"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_nfc"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_nfc"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_nfc.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_nfc"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_nfc/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Queries NFC readiness and reads one NDEF text or URI record (up to 4 KiB) from a user-presented tag. iOS uses Core NFC; Android uses reader mode on the host activity. macOS, Windows, Linux, the terminal, and the browser are unavailable. Writing tags, ISO-DEP APDUs, and background delivery are out of scope.

Enable `guest` for native bridge calls. Select `nfc` in `native.plugins`. iOS needs an app-owned `NFCReaderUsageDescription` and the Near Field Communication Tag Reading capability; Android needs the `NFC` permission. Generated hosts copy those declarations from the crate metadata where supported.

Cancellation stops the caller waiting for a tag; the OS sheet may still need the user to dismiss it. Automated tests use injected readers and never require hardware.

## How it fits

<!-- {=p_nfcSummary} -->

Add `leptoskit_plugin_nfc` as a direct dependency and select the `nfc` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::Nfc` rather than resolving the plugin on every call. A successful read returns one bounded NDEF text or URI record; it never writes tags or claims background tag delivery.

<!-- {/p_nfcSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_nfc::{Availability, guest::Nfc};

let nfc = Nfc::new()?;
if nfc.availability().await? == Availability::Ready {
    let record = nfc.read().await?;
    println!("{} {:?}", record.value(), record.kind());
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_nfc)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
