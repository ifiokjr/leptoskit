---
leptoskit_plugin_localization: major
---

# leptoskit_plugin_localization: locale preferences

`leptoskit_plugin_localization` is the leptoskit plugin for ordered preferred locales, language and region codes, calendar, timezone, first weekday, and clock preference.

Add `leptoskit_plugin_localization` as a direct dependency and select the `localization` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Localization` rather than resolving the plugin on every call. iOS, macOS, and Android read the system settings, desktop Rust hosts register `desktop::Localization`, and the browser calls `web::snapshot()`. Each snapshot reflects current settings and no global locale is mutated; there are no change events yet, so re-query after the app resumes.
