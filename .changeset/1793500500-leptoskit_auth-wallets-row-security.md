---
"release": minor
---

# Wallet rules run as row-level security

`cx.wallets()` is now a list setting, so a rule such as `vault.treasury.in_list(cx.wallets())` also works with `[db] policies = "rls"`. Previously, exporting the schema refused it. Only the auth layer can build a `Wallets` value, and only beside a signed-in person; read its addresses with `Wallets::addresses()`.
