---
"release": minor
---

# Async storage adapters for store and secure-store plugins

`leptoskit_plugin_store` and `leptoskit_plugin_secure_store` now implement `leptoskit_use::AsyncStorage` behind an `async-storage` feature, so `use_async_storage` persists shared state through the store plugin's settings document on native and browser builds, and through the OS credential store for small secrets on desktop and native guests. Adapter writes commit before signals update, failures preserve the last committed value, and storage keys that cannot serve as credential names are stored under SHA-256 digests. The browser suite exercises the adapter against real local storage, including rejected writes.
