<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_auth

> Sign-in for leptoskit apps: the auth schema pack, sessions, email codes, and the signed-in user that access policies read.

Sign-in lives in your app's own database, as the `auth` schema pack. Add it next to your tables:

```rust,ignore
leptoskit::db::packs!(leptoskit_auth);
```

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

The tables, `SignedIn`, and the `Public*` row types build everywhere, including the browser. `Auth` needs the `server` feature and a database: `postgres` or `sqlite`.
