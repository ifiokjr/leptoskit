---
leptoskit_plugin_image_manipulator: breaking
---

# leptoskit_plugin_image_manipulator: image transforms

`leptoskit_plugin_image_manipulator` is the leptoskit plugin for bounded JPEG and PNG decode, resize, crop, rotate, flip, and encode using one Rust implementation.

Add `leptoskit_plugin_image_manipulator` as a direct dependency and select the `image-manipulator` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Native calls run on the Rust worker so a batch takes one bridge round trip; browser calls use the same engine compiled to Wasm, and large jobs belong in a Web Worker.
