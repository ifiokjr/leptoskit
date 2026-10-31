---
"release": minor
---

# Solana wallets, and merging accounts with their data

`leptoskit::auth` links Solana wallets to accounts (the `auth-solana` feature on your server, plus `[auth] origin`), and merges two accounts with the other account's data.

- **Link a wallet** with `link_wallet()`: the person's wallet signs a Sign-In With Solana message bound to your site, with a single-use nonce that expires in five minutes, and the server checks it byte for byte and as ed25519. In the browser, any wallet that supports the Wallet Standard works; `use_wallet_apps()` lists them.
- **Wallets are private, and don't sign in, until the person says so.** `update_wallet()` shows one on their profile (`auth.public_wallets(account)`) or lets it sign in, with `[auth.solana] sign_in = true` on the server. `wallet_sign_in()` is one factor, so two-step verification still applies. `unlink_wallet()` removes one.
- **Fresh proofs:** `auth.require_wallet_proof(&me, &address, window)` answers `proof_required` until the wallet signs again with `prove_wallet()`, for payouts and other actions tied to the address.
- **Rules can name a wallet's owner:** `vault.treasury.in_list(cx.wallets())`.
- **New scalars:** `leptoskit::db::Pubkey` and `Signature`, base58 text that must decode to exactly 32 or 64 bytes. Scalars can run their own check with `#[db(validate_with = …)]`.
- **Merge with the data:** `MergeMode::BringData` moves the other account's rows into this one, each table as its `#[db::table(on_account_merge = …)]` strategy says: `reassign` (the default), `reassign_skip_conflicts`, `keep_newest`, `keep_survivor`, or `custom` with your `auth::OnMerge`. The merge offer previews its data (`MergeOffer::data`), and a failure changes nothing. `leptoskit db check` fails until every table that could collide declares a strategy.
- `lk generate auth` adds wallets and the merge preview to the security screen, and wallet buttons to the sign-in screen.
- The functions exist on every platform. Native apps manage linked wallets, and answer `unsupported` for asking a wallet to sign until a native wallet adapter exists.
