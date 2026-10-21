# leptoskit_plugin_localization

Ordered preferred locales, language and region codes, calendar identifier, timezone, first weekday, and clock preference. All snapshots reflect the current platform state. No process locale is changed and no locale snapshot is cached.

```rust,ignore
let localization = leptoskit_plugin_localization::guest::Localization::new()?;
let preferences = localization.snapshot().await?;
```

Add this crate as a direct app dependency and `"localization"` to `native.plugins` for automatic Apple/Android registration. Rust desktop hosts register `desktop::Localization`; browser applications call `web::snapshot()`.

| Platform                 | Source and limits                                                                                                 |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------- |
| iOS/macOS native         | Foundation preferred languages, current calendar, timezone and hour cycle                                         |
| Android                  | App configuration LocaleList, ICU calendar, system timezone and 24-hour setting                                   |
| Rust macOS/Windows/Linux | OS preferred locales through sys-locale and IANA timezone; calendar, weekday and clock preferences remain unknown |
| Browser                  | Navigator.languages and Intl.Locale/DateTimeFormat; first weekday stays unknown without Intl week-info support    |

Region is never inferred from language alone. First weekday is Monday=1 through Sunday=7. Gregorian identifiers are normalized to `gregory`; other identifiers retain the platform's vocabulary. Browser clock preference comes from Intl locale resolution and may differ from a separate OS clock setting. Desktop locale tags that cannot be parsed as Unicode language identifiers remain intact with unknown language/region fields.

Re-query when the app resumes to reflect settings changes. Locale change events, text direction, number separators, measurement systems, currencies and temperature preferences are not yet implemented. The current port covers the core preference snapshot from [Expo Localization](https://docs.expo.dev/versions/latest/sdk/localization/).
