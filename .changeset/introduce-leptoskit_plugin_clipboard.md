---
leptoskit_plugin_clipboard: breaking
---

# leptoskit_plugin_clipboard: text clipboard access

`leptoskit_plugin_clipboard` is the leptoskit plugin for system text clipboard access - read, write, and clear - on iOS, Android, macOS, Windows, Linux, and in the browser.

Add `leptoskit_plugin_clipboard` as a direct dependency and select the `clipboard` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Clipboard` rather than resolving the plugin on every call. Only plain text is supported; browser operations need a secure context and may need a user gesture.
