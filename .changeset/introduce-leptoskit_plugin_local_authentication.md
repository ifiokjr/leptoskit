---
leptoskit_plugin_local_authentication: breaking
---

# leptoskit_plugin_local_authentication: biometrics

`leptoskit_plugin_local_authentication` is the leptoskit plugin for local device-owner authentication through Apple `LAContext` and Android `BiometricPrompt`.

Add `leptoskit_plugin_local_authentication` as a direct dependency and select the `local-authentication` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::LocalAuthentication` rather than resolving the plugin on every call. This is not account identity or server authorization; the plugin reports explicit terminal outcomes and never invents a success.
