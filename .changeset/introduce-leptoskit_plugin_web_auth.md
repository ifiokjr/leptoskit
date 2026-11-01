---
leptoskit_plugin_web_auth: breaking
---

# leptoskit_plugin_web_auth: system-browser sign-in

`leptoskit_plugin_web_auth` is the leptoskit plugin for sign-in sessions in the system browser - ASWebAuthenticationSession on Apple platforms, Auth Tab on Android, and a loopback redirect on desktop.

Add `leptoskit_plugin_web_auth` as a direct dependency and select the `web-auth` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::WebAuth` rather than resolving the plugin on every call. It follows RFC 8252: the provider page never loads in a web view inside the app.
