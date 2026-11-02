<h1 align="center">leptoskit_plugin_localization</h1>

<p align="center">
  <strong>leptoskit_plugin_localization</strong> is the leptoskit plugin for ordered preferred locales, language and region codes, calendar, timezone, first weekday, and clock preference.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_localization"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_localization"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_localization.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_localization"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_localization/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Ordered preferred locales, language and region codes, calendar identifier, timezone, first weekday, and clock preference. All snapshots reflect the current platform state. No process locale is changed and no locale snapshot is cached.

Add this crate as a direct app dependency and `"localization"` to `native.plugins` for automatic Apple/Android registration. Rust desktop hosts register `desktop::Localization`; browser applications call `web::snapshot()`.

| Platform                 | Source and limits                                                                                                 |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------- |
| iOS/macOS native         | Foundation preferred languages, current calendar, timezone and hour cycle                                         |
| Android                  | App configuration LocaleList, ICU calendar, system timezone and 24-hour setting                                   |
| Rust macOS/Windows/Linux | OS preferred locales through sys-locale and IANA timezone; calendar, weekday and clock preferences remain unknown |
| Browser                  | Navigator.languages and Intl.Locale/DateTimeFormat; first weekday stays unknown without Intl week-info support    |

Region is never inferred from language alone. First weekday is Monday=1 through Sunday=7. Gregorian identifiers are normalized to `gregory`; other identifiers retain the platform's vocabulary. Browser clock preference comes from Intl locale resolution and may differ from a separate OS clock setting. Desktop locale tags that cannot be parsed as Unicode language identifiers remain intact with unknown language/region fields.

Re-query when the app resumes to reflect settings changes. Locale change events, text direction, number separators, measurement systems, currencies and temperature preferences are not yet implemented. The current port covers the core preference snapshot from [Expo Localization](https://docs.expo.dev/versions/latest/sdk/localization/).

## How it fits

Add `leptoskit_plugin_localization` as a direct dependency and select the `localization` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Localization` rather than resolving the plugin on every call. Each snapshot reflects current platform state, no global locale is mutated, and a host should re-query after the application resumes.

## Use it

```rust,ignore
let localization = leptoskit_plugin_localization::guest::Localization::new()?;
let preferences = localization.snapshot().await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_localization)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
