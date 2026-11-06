---
"release": minor
---

# Sign which origins serve a deployment

Add `leptoskit verify keygen`, `leptoskit verify sign`, and `leptoskit verify deployment`. A project signs a deployment policy listing its origins and the verified releases they may serve, and serves it at `/.well-known/leptoskit/deployment.json`. Anyone can check an origin against the project's published key: a copied website fails because its origin is not listed, a forged policy fails its signature, and `--state` refuses an older or conflicting policy. The check does not show what a server actually runs.
