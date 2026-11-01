---
leptoskit_plugin_device: breaking
---

# leptoskit_plugin_device: device information

`leptoskit_plugin_device` is the leptoskit plugin for device and operating system information without persistent identifiers or user-assigned names.

Add `leptoskit_plugin_device` as a direct dependency and select the `device` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Device` rather than resolving the plugin on every call. Native guests, Rust desktop hosts, and browsers each return a validated binary snapshot with the fields that platform can actually provide.
