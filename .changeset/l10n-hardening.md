---
"release": patch
---

# Harden translations: names, variants, scripts, quoting

`leptoskit l10n` now rejects, with the file and line, ARB files that used to generate a module that misbehaved or did not compile: a key that would become one of the `L10n` handle's own methods (`locale`, `set_locale`, `messages`, `clone`, `into`, and the rest), which `l10n.locale()` would call instead of the message; a key or placeholder named `gen`, reserved from Rust 2024; an `=N` form larger than its placeholder's type holds; and quoted text that reads as a placeholder, such as `'{name}'`, which a Flutter project without `use-escaping` means as the placeholder. A key that is not an identifier says why instead of calling it a keyword, and problems are listed by file and then by line. Outside a plural, `'#'` now stays as written, as in ICU.

Locales with variants, such as `ca-ES-valencia` next to `ca-ES`, each get their own messages, and a placeholder can no longer shadow a locale's plural rules. Forms with the same text share one `match` arm or `if` branch, a select or plural with only `other` is its text, and a placeholder a translation leaves unread is named `_name`, so the generated module passes clippy's pedantic lints. Message text holding backticks shows as code in the generated documentation. `leptoskit l10n --check` also stays green after rustfmt rewraps the module's documentation or splits its long strings.

Locale negotiation fills in the script CLDR expects for languages written in more than one, so `zh-TW` picks `zh-Hant` over `zh-Hans`, and the docs list where it chooses differently from Flutter. `accept_language` leaves out entries whose quality value RFC 9110 does not allow (`q=NaN`, `q=5`), and `browser_locales` is exported with or without the `web` feature, returning an empty list wherever there is no browser to ask.
