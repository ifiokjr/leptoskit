---
leptoskit_plugin_permissions: major
---

# leptoskit_plugin_permissions: permissions

`leptoskit_plugin_permissions` is the leptoskit plugin for explicit camera and microphone permission queries and requests.

Add `leptoskit_plugin_permissions` as a direct dependency and select the `permissions` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Permissions` rather than resolving the plugin on every call. iOS, macOS, and Android use the system permission dialogs and browser apps call the `web` module; Windows and Linux have no backend. Querying status never shows a prompt, so request only after the user chooses a feature. Other permissions are not covered here: the location and notifications plugins request their own.
