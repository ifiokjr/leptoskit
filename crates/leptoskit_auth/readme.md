<h1 align="center">leptoskit_auth</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_auth</strong> is sign-in for leptoskit apps: the auth schema pack, sessions, email codes, recovery, passkeys, providers, and the signed-in user that access policies read.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_auth"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_auth"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_auth.svg"></a>
  <a href="https://docs.rs/leptoskit_auth"><img alt="docs.rs" src="https://docs.rs/leptoskit_auth/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Sign-in lives in your app's own database, as the `auth` schema pack. Add it next to your tables.

Key your user table by the account, and create its row when an account is created:

```rust,ignore
#[db::table(name = "users", public)]
pub struct User {
	#[db(on_delete = cascade)]
	pub id: Id<auth::Account>,
	pub name: String,
}

impl auth::OnSignUp for User {
	async fn on_sign_up(tx: &db::Tx, account: Id<auth::Account>, profile: &auth::Profile) -> auth::Result<()> {
		NewUser { id: account, name: "New user".into() }.insert(tx).await?;
		Ok(())
	}
}
```

Then sign people in with a code sent to their email address. The same flow signs up an address that has no account, and every address gets the same answer:

```rust,ignore
let auth = auth::Auth::builder(&db, auth::Keys::from_env("AUTH_ENCRYPTION_KEYS")?)
	.mailer(my_mailer)
	.on_sign_up::<User>()
	.build()?;

auth.send_email_code(&address, &request).await?;
let sign_in = auth.verify_email_code(&address, &code, &request).await?;   // sign_in.token goes to the device
let signed_in = auth.resolve(&sign_in.token).await?;                      // on every request
```

- **Sessions** are opaque 256-bit tokens, and the database keeps only their SHA-256 hash. They have idle and absolute lifetimes, and can be signed out, revoked, or rotated, all taking effect at once.
- **Codes** are keyed hashes. Each works once, for 10 minutes, for its own address, and allows five guesses.
- **Rate limits** apply per address and per IP address, with counters that concurrent attempts can't overrun.
- **The audit trail** records sign-ups, sign-ins, failures and revocations, with the IP address and user agent, and never credentials.

Policies read the signed-in person through `cx.signed_in`, which matches nothing for a signed-out request:

```rust,ignore
#[db::policy(all, |post, cx| cx.signed_in(|me| post.author_id.eq(me.user_id())))]
```

The pack's own tables show each person only their own rows, and only this crate writes them.

With the `solana` feature, people link Solana wallets with a signed Sign-In With Solana message, re-prove them before sensitive actions, and may sign in with them. Policies name a wallet's owner with `cx.wallets()`:

```rust,ignore
#[db::policy(update, |vault, cx| vault.treasury.in_list(cx.wallets()))]
```

With the `oauth` feature, people sign in with any of twenty built-in providers, such as Google, GitHub, Discord, or Slack, or any OpenID Connect issuer; identity is the provider's subject, never an email address. A provider configured with `store_tokens` keeps its tokens, sealed, for the app to call its API, and `Auth::provider_access_token` refreshes them.

With the `password` feature and `[auth.passwords] enabled`, people may also sign in with a password: Argon2id, NIST SP 800-63B rules, and resets through emailed codes.

Two accounts can be merged, moving only the sign-in methods or bringing the other account's data along, as each table's `on_account_merge` says.

The tables, `SignedIn`, and the `Public*` row types build everywhere, including the browser. `Auth` needs the `server` feature and a database: `postgres` or `sqlite`.

## How it fits

It owns the server-side session and the shared client, and uses the passkey and web-auth plugins for native sign-in. `leptoskit_idp` turns the same accounts into an OIDC provider.

## Use it

```rust,ignore
leptoskit::db::packs!(leptoskit_auth);
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_auth)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Sign in](https://ifiokjr.github.io/leptoskit/guide/23-sign-in.html)
