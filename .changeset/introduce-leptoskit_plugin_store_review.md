---
leptoskit_plugin_store_review: breaking
---

# leptoskit_plugin_store_review: store review requests

`leptoskit_plugin_store_review` is the leptoskit plugin for owned-window in-app store review requests through StoreKit and Google Play.

Add `leptoskit_plugin_store_review` as a direct dependency and select the `store-review` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::StoreReview` rather than resolving the plugin on every call. Acceptance never proves a dialog appeared or a review was submitted, and the plugin never substitutes another application's scene.
