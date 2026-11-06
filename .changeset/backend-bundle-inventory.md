---
"release": minor
---

# Record and check offline backend bundles

Add `leptoskit verify record` and `leptoskit verify check` to inventory prepared backend source, dependencies, infrastructure, build declarations, secret names and executable files. Checks require an independently trusted inventory SHA-256 and reject changed, missing or extra entries and symbolic links. These commands check local bundle integrity; they do not attest the build or verify a live backend.
