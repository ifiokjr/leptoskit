---
"release": minor
---

# Rename leptoskit_fns to leptoskit_rpc

The crate's job is publishing the RPC — describing it and generating clients — so the name now says so: `leptoskit_fns` becomes `leptoskit_rpc`, with `leptoskit_rpc_codegen` and `leptoskit_rpc_macros` alongside. The `leptoskit` meta crate's `fns` feature and module become `rpc`, the CLI command becomes `leptoskit rpc` (spec/list/generate unchanged), the configuration section becomes `[rpc]`, and the export environment variables become `LEPTOSKIT_RPC_*`. The retry/query layer the crate started as keeps its API under the new name.
