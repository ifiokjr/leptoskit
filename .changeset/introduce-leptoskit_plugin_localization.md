---
leptoskit_plugin_localization: breaking
---

# leptoskit_plugin_localization: locale preferences

`leptoskit_plugin_localization` is the leptoskit plugin for ordered preferred locales, language and region codes, calendar, timezone, first weekday, and clock preference.

Add `leptoskit_plugin_localization` as a direct dependency and select the `localization` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Localization` rather than resolving the plugin on every call. Each snapshot reflects current platform state, no global locale is mutated, and a host should re-query after the application resumes.
