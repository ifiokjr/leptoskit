---
leptoskit_plugin_document_picker: major
---

# leptoskit_plugin_document_picker: document import

`leptoskit_plugin_document_picker` is the leptoskit plugin for bounded document import through the iOS document picker, macOS open panel, Android Storage Access Framework, or an HTML file input.

Add `leptoskit_plugin_document_picker` as a direct dependency and select the `document-picker` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and call `pick`, browser apps call `web::pick` from a user action, and Rust desktop hosts on macOS, Windows, and Linux can call `desktop::pick` with a parent window. The user picks one document; the app receives its name, its MIME type when known, and owned bytes up to 4 MiB, and never gets a reusable path into the document provider.
