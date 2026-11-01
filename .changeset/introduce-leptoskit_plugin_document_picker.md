---
leptoskit_plugin_document_picker: breaking
---

# leptoskit_plugin_document_picker: document import

`leptoskit_plugin_document_picker` is the leptoskit plugin for bounded document import through the iOS document picker, macOS open panel, Android Storage Access Framework, or an HTML file input.

Add `leptoskit_plugin_document_picker` as a direct dependency and select the `document-picker` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. The user picks the document; the app receives owned bytes under a size limit and never gets a reusable path into the document provider.
