---
leptoskit_plugin_logging: major
---

# leptoskit_plugin_logging: structured logging

`leptoskit_plugin_logging` is the leptoskit plugin for structured, bounded application logging: records with a severity, a target, a message, and named fields, written as JSON lines to rotated files.

Add `leptoskit_plugin_logging` as a direct dependency and select the `logging` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and write through `guest::Logger`. Native apps write to `<app data>/logs`, keeping four files of up to 1 MiB by default, and the browser logs to the console without persisting anything. There is no global logger, remote backend, or automatic redaction, and guests cannot choose where logs go.
