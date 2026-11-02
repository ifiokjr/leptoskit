---
leptoskit_plugin_dialog: breaking
---

# leptoskit_plugin_dialog: native dialogs

`leptoskit_plugin_dialog` is the leptoskit plugin for asynchronous native file, directory, save, and message dialogs on desktop, plus browser file selection.

Add `leptoskit_plugin_dialog` as a direct dependency and select the `dialog` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Desktop calls need an initialized application event loop and a parent window; the browser adapter uses the File System Access API with an explicit byte limit.
