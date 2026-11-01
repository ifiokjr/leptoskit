---
leptoskit_plugin_os: breaking
---

# leptoskit_plugin_os: host platform identity

`leptoskit_plugin_os` is the leptoskit plugin for the host operating system and CPU architecture as seen from inside a Wasm guest.

Add `leptoskit_plugin_os` as a direct dependency and select the `os` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. The values come from compile-time constants of the host build, so the guest sees the real platform even though its own target is wasm32.
