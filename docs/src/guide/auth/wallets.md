# Solana wallets

Some apps need to know that a person really holds a Solana address: to pay out to it, to show that a treasury belongs to a team, or to let only the holder of a wallet change something on-chain-related. Typing an address in proves nothing. Linking a wallet asks the person's wallet app to sign a short message for your site, which only the holder of the key can do. Nothing is sent on-chain and nothing costs a fee.

Once linked, a wallet is a proven part of the person's account. They can also let it sign them in, one wallet at a time.

## Turn it on

Wallets need the `auth-solana` feature on your server's `leptoskit` dependency, and `[auth] origin`: its host is the site every message names, which wallets compare with the page asking.

```toml
[auth]
origin = "https://myapp.com"

[auth.solana]
sign_in = true # let wallets sign in, once their owner turns that on; off by default
chain_id = "mainnet" # the cluster messages name; "devnet" or "testnet" for testing
```

`Auth::from_config` reads `[auth.solana]`; `AuthBuilder::solana(auth::SolanaOptions { … })` does the same in code. Without the feature, the functions below still exist, and the server answers `unsupported`.

## Link a wallet

From the security screen, the person picks one of the wallet apps their browser has, and approves the message in it:

```rust
use leptoskit::auth::WalletLinked;
use leptoskit::auth::client::LinkWalletInput;
use leptoskit::auth::client::link_wallet;
use leptoskit::auth::client::use_wallet_apps;
use leptoskit::auth::client::use_wallets;

let apps = use_wallet_apps();       // [WalletApp { name: "Phantom", icon: "data:…", sign_in: true }, …]
let wallets = use_wallets();        // the linked ones: address, label, can_sign_in, is_public, …
let link = use_mutation(link_wallet());
link.mutate(LinkWalletInput { wallet: "Phantom".into(), label: Some("Treasury".into()) });
```

The wallet shows a message like this, and signs it:

```text
myapp.com wants you to sign in with your Solana account:
9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM

Link this wallet to your account on My App.

URI: https://myapp.com
Version: 1
Chain ID: mainnet
Nonce: 4WQ5WmPxpHdRqv8a3NhVF2
Issued At: 2026-10-30T12:00:00Z
Expiration Time: 2026-10-30T12:05:00Z
Request ID: link
```

That's Sign-In With Solana, the format wallets know. Wallets that support the Wallet Standard's sign-in, such as Phantom and Solflare, show it as a sign-in request for your site; wallets that only sign text are asked to sign the same message.

`link_wallet()` answers:

| Answer             | Meaning                                                                               |
| ------------------ | ------------------------------------------------------------------------------------- |
| `Linked(wallet)`   | It's linked. It's private, and can't sign in until the person turns that on           |
| `AlreadyLinked(_)` | It was linked already; its proof is fresh now                                         |
| `Merge`            | It's linked to another account here that it can sign in to: see [merging](merging.md) |

A wallet belongs to one account. If it's linked to another account that it can't sign in to, linking fails with `wallet_in_use`: signing with it proves the person holds the key, not that they own that account. Linking needs a recent sign-in or step-up, like every change to how an account signs in, and the account's email gets a note.

## Manage linked wallets

```rust
use leptoskit::auth::UpdateWallet;
use leptoskit::auth::client::unlink_wallet;
use leptoskit::auth::client::update_wallet;

update_wallet().mutate(UpdateWallet { id: wallet.id, label: Some("Cold storage".into()), can_sign_in: None, is_public: Some(true) });
unlink_wallet().mutate(wallet.id);
```

- **Private by default.** `is_public: Some(true)` shows a wallet on the person's profile. On the server, `auth.public_wallets(account)` returns only those, as `PublicWallet { chain, address, label }`, for a profile page. Linking never announces a wallet by itself.
- **Signing in is off by default.** Turning it on needs a recent sign-in or step-up, and emails the person.
- **Unlinking** needs a recent sign-in or step-up, and the account must keep a way to sign in.

## Sign in with a wallet

When `[auth.solana] sign_in` is on and the person turned it on for a wallet:

```rust
use leptoskit::auth::client::SignInStep;
use leptoskit::auth::client::wallet_sign_in;

let sign_in = use_mutation(wallet_sign_in());
sign_in.mutate("Phantom".to_owned());   // SignInStep::SignedIn, or SecondFactor
```

A wallet signature is one factor, like an emailed code, because a wallet key doesn't verify who is holding the device. An account with two-step verification answers `SecondFactor`, and `complete_sign_in()` finishes as after an email. A wallet that no account lets sign in answers `unknown_wallet`.

## Ask for a fresh proof

A wallet linked a year ago may have changed hands. Before a payout or another action tied to the address, ask for a signature from the last few minutes:

```rust
// On the server, in a handler or server function.
let wallet = auth.require_wallet_proof(&me, &payout.address, Duration::from_secs(120)).await?;
```

It fails with `proof_required` when the wallet's last proof is older. The client answers with `prove_wallet()`, which asks that wallet to sign a message naming its address, then retries:

```rust
use leptoskit::auth::client::ProveWalletInput;
use leptoskit::auth::client::prove_wallet;

prove_wallet().mutate(ProveWalletInput { wallet: "Phantom".into(), address: wallet.address.clone() });
```

## Rules that name a wallet's owner

Policies read the signed-in person's linked wallets as `cx.wallets()`, which is empty for everyone else:

```rust
use leptoskit::auth::AuthContext;

#[db::table(name = "vaults")]
#[db::policy(select, update, |vault, cx| vault.treasury.in_list(cx.wallets()))]
pub struct Vault {
	pub id: Id<Vault>,
	pub treasury: Pubkey,
}
```

The layer reads the wallets with the session, so every request's rules see them.

## On every platform

| Platform                                                               | Wallets                                                                                                                                                                                      |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Web                                                                    | Everything above, with any wallet the browser has that supports the Wallet Standard                                                                                                          |
| iOS, Android, macOS, Windows, Linux, and HarmonyOS (experimental) apps | `use_wallets()`, `update_wallet()`, `unlink_wallet()`, and `use_wallet_settings()` work; `use_wallet_apps()`, `link_wallet()`, `prove_wallet()`, and `wallet_sign_in()` answer `unsupported` |

Native apps have no Wallet Standard to reach wallet apps through yet. The functions are the same on every platform, so your code doesn't change when a native wallet adapter arrives; until then, people link wallets in the browser, and manage them anywhere. The generated sign-in screen shows wallet buttons only where there are wallet apps to show.

## What message signing can't prove

Signing a message proves control of a key pair. It can't prove control of a program-derived address or a multisig such as Squads, which have no key to sign with, and some hardware wallets can't sign messages at all. Proving those with a transaction from the address is planned, and not built yet.

## How it's kept safe

- **Bound to your site.** Every message names your origin's host and URI. Wallets warn when a page asks for a message for another site, so a phishing page can't relay yours.
- **Works once, for five minutes.** The message's nonce is 128 random bits, kept on the server, encrypted, with the message's fields. It's used up before the answer is checked, so a replayed signature, or a second answer to the same request, fails.
- **Byte for byte.** The signed text must be exactly the message issued for the address that signed it, and parse strictly as Sign-In With Solana. The signature must verify as strict ed25519, which refuses weak keys and reshaped signatures.
- **One browser.** A wallet sign-in started in a browser finishes only there: the request is matched by a random token in an `HttpOnly` cookie.
- **Recorded.** Linking, proofs, refusals, and changes are audit events, and the person is emailed when a wallet is linked, unlinked, or allowed to sign in. The latest signed message is kept as proof.
