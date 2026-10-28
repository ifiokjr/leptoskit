---
"release": patch
---

# Encode untrusted RPC metadata in generated clients

RPC client generation now encodes titles and versions in source comments, Python module strings, and package manifests. CLI help text prefixes every imported documentation line as a comment. Untrusted specification metadata can no longer add Cargo dependencies or inject generated commands through these fields.
