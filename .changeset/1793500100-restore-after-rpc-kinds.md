---
"release": patch
---

# Restore sign-in routes and facade features lost in the operation kinds merge

The merge that added `#[rpc::query]` and `#[rpc::mutation]` also put back older copies of shared files, which removed the sign-in routes and client calls for passkeys, providers, account merging, and wallets, the `db-studio` and `db-geo` features, and the identity provider's features in `leptoskit`. They are back, alongside the new RPC sign-in support.

The `rpc.receipts` table's policy now reads its caller through a setting, so apps using operations can also turn on `[db] policies = "rls"`.
