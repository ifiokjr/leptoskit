---
"release": minor
---

# Embed wasm-bindgen 0.2.129

The CLI now binds clients with wasm-bindgen 0.2.129, and `leptoskit new` pins the client to it. wasm-bindgen only binds wasm built against the same schema, and since 0.2.128 its schema version no longer moves with every release, so `leptoskit build` and `leptoskit doctor` now accept any locked wasm-bindgen from the embedded schema's release (0.2.128) up to the embedded 0.2.129, and name the exact `cargo update --precise` fix otherwise. Projects on 0.2.100 need that update.
