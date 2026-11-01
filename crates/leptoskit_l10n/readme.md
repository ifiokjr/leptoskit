<h1 align="center">leptoskit_l10n</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_l10n</strong> provides typed translations in the style of Flutter's `gen_l10n`: locales, negotiation, and plural rules behind a generated messages module.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_l10n"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_l10n"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_l10n.svg"></a>
  <a href="https://docs.rs/leptoskit_l10n"><img alt="docs.rs" src="https://docs.rs/leptoskit_l10n/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Messages live in one ARB file per locale (`locales/en.arb`, `locales/pt-BR.arb`), and `leptoskit l10n` writes `src/l10n.rs` from them: a `Messages` trait with one method per message, typed by its placeholders; one implementation per locale, with untranslated messages falling back to the template; and the handle an app reads them through. This crate is the runtime behind that module.

- `Locale` is a language tag (`en`, `pt-BR`, `zh-Hant-TW`, `ca-ES-valencia`), parsed, canonicalised, and compared case-insensitively.
- `negotiate` picks the supported locale that serves a user's preferences best, in a way based on Flutter's: the same tag, then the same language and script, then language and region, then language alone. A tag without a script gets the one CLDR expects for its region, so `zh-TW` picks `zh-Hant`; the [localization guide](https://ifiokjr.github.io/leptoskit/guide/19-localization.html#how-a-locale-is-chosen) lists where it chooses differently from Flutter.
- `PluralRules` carry CLDR's cardinal rules for every language it covers, so `{count, plural, one {# item} other {# items}}` picks the right form in Polish and Arabic as well as English.
- `accept_language` reads a request's `Accept-Language` header on the server, leaving out entries with a quality value RFC 9110 does not allow. `browser_locales` reads `navigator.languages` in the browser with the `web` feature, and returns an empty list everywhere else.
- `L10n` holds the locale in use in a signal and provides the translations to the component tree; a view that reads a message renders again when the locale changes.

The [localization guide](https://ifiokjr.github.io/leptoskit/guide/19-localization.html) covers the setup: the `[l10n]` table, the ARB files, the generated module, and where the preferred locales come from on web, iOS, Android, macOS, Windows, Linux, and HarmonyOS.

## How it fits

Messages live in one ARB file per locale; `leptoskit l10n` writes the typed module, and app code calls getters instead of parsing ARB at runtime. It works in SSR, the browser, and native guests.

## Use it

```rust
let l10n = use_l10n();
view! {
	<Text>{move || l10n.cart_items(count.get())}</Text>
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_l10n)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Localization](https://ifiokjr.github.io/leptoskit/guide/19-localization.html)
