---
leptoskit_plugin_keep_awake: breaking
---

# leptoskit_plugin_keep_awake: display wake locks

`leptoskit_plugin_keep_awake` is the leptoskit plugin for scoped display wake locks with cancellation-safe ownership.

Add `leptoskit_plugin_keep_awake` as a direct dependency and select the `keep-awake` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::KeepAwake` rather than resolving the plugin on every call. iOS preserves and restores the idle timer, Android uses the activity's keep-screen-on flag, desktop uses OS assertions, and the browser uses the Screen Wake Lock API; none of them promise background execution.
