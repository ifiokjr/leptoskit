---
leptoskit_plugin_image_manipulator: major
---

# leptoskit_plugin_image_manipulator: image transforms

`leptoskit_plugin_image_manipulator` is the leptoskit plugin for bounded JPEG and PNG decode, resize, crop, rotate, flip, and encode using one Rust implementation.

Add `leptoskit_plugin_image_manipulator` as a direct dependency and select the `image_manipulator` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests use `default-features = false, features = ["guest"]` and call `guest::ImageManipulator`, which skips the image codecs; native calls run on the Rust worker so a batch takes one bridge round trip. Browser calls use the same engine compiled to Wasm, and large jobs belong in a Web Worker. Images are limited to 4 MiB, 8,192 pixels per side, and 16 operations per request.
