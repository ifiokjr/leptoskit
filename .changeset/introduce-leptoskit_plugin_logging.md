---
leptoskit_plugin_logging: breaking
---

# leptoskit_plugin_logging: structured logging

`leptoskit_plugin_logging` is the leptoskit plugin for structured, bounded application logging with explicit sinks and rotation.

Add `leptoskit_plugin_logging` as a direct dependency and select the `logging` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. There is no global logger or remote backend; the host chooses the log directory and limits, and the guest protocol stays bounded independently of JSON-lines output.
