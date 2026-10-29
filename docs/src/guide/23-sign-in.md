# Sign-in

Sign-in is the part of an app where small mistakes are the most expensive. Session tokens get stored as they are, so a leaked database backup signs everyone in. A form says "no account with that email", which tells an attacker whose accounts exist. A code can be guessed because nothing counts the guesses. And one server function forgets to check who is asking.

`leptoskit::auth` handles all of these in one place, in your app's own database. It keeps only hashes of tokens and codes, gives the same answer for every address, and limits every attempt. It also tells your access policies who is signed in, so each table's rules apply on every query.

## Add the auth tables

Sign-in keeps its tables in your database as a [schema pack](22-database.md#tables-that-come-from-a-library). Turn on the `auth` feature next to your database feature, then add the pack in the crate that declares your tables:

```toml
leptoskit = { version = "…", features = ["auth", "db-postgres"] }
```

```rust
leptoskit::db::packs!(leptoskit_auth);
```

Your next `leptoskit db migration create` adds the tables, in the `auth` namespace:

| Table         | What it holds                                                                    |
| ------------- | -------------------------------------------------------------------------------- |
| `accounts`    | One row per person: when they joined, and whether the account is disabled.       |
| `emails`      | The addresses each account has proved it receives mail at.                       |
| `sessions`    | Signed-in devices: a hash of each token, its lifetimes, and a label.             |
| `challenges`  | Codes in flight, as keyed hashes. They expire and work once.                     |
| `events`      | The audit trail: sign-ups, sign-ins, failures, and revocations, with IP address. |
| `rate_limits` | Attempt counters, keyed by a hash, so the table holds no addresses.              |

## Connect your user table

Give your user table the account's key, so `Id<auth::Account>` names the user too. Deleting an account then deletes its user:

```rust
#[db::table(name = "users", public)]
pub struct User {
	#[db(on_delete = cascade)]
	pub id: Id<auth::Account>,
	pub name: String,
}
```

Create the user's row when the account is created, with `OnSignUp`. It runs inside the transaction that creates the account, so if it fails, nothing is created:

```rust
impl auth::OnSignUp for User {
	async fn on_sign_up(tx: &db::Tx, account: Id<auth::Account>, profile: &auth::Profile) -> auth::Result<()> {
		let name = profile.name.clone().unwrap_or_else(|| "New user".into());
		NewUser { id: account, name }.insert(tx).await?;
		Ok(())
	}
}
```

## Start the service

Build one `Auth` when the server starts, and share it:

```rust
let auth = auth::Auth::builder(&db, auth::Keys::from_env("AUTH_ENCRYPTION_KEYS")?)
	.app_name("My App")
	.mailer(my_mailer)
	.on_sign_up::<User>()
	.build()?;
```

`AUTH_ENCRYPTION_KEYS` holds the server's keys, newest first, as `id:key` pairs, where each key is 32 random bytes in base64url:

```sh
echo "AUTH_ENCRYPTION_KEYS=$(date +%Y-%m):$(openssl rand 32 | basenc --base64url | tr -d =)"
```

The newest key hashes new codes, and any listed key checks one. To rotate, put a new key first and keep the old one until the codes it hashed have expired. Keep the variable in your secret store, never in `leptoskit.toml`.

A mailer is anything that implements `auth::Mailer`, which takes a few lines over your email provider's HTTP API. In development and tests, `auth::Mailbox` keeps every email instead of sending it.

## Sign in with an emailed code

One flow both signs in and signs up. Ask for a code, and the person receives one by email:

```rust
auth.send_email_code(&address, &request).await?;
```

The answer is the same for every valid address, whether or not it has an account, so the form can't be used to find out who has signed up. When the person types the code, check it:

```rust
let sign_in = auth.verify_email_code(&address, &code, &request).await?;
// sign_in.token goes to the device; sign_in.signed_up says whether the account is new
```

A code proves the address. If the address has an account, this signs in to it. If it doesn't, this creates the account and runs your `OnSignUp`. The rules on codes:

- A code is eight letters and digits, such as `K7QM-R2XD`. Case, spaces, and the dash don't matter.
- It works once, for 10 minutes, for its own address only. Asking for a new code retires the old one.
- After five wrong guesses it stops working. A wrong code, an expired code, and an address with no code all fail the same way, with `AuthError::InvalidCode`.

`request` is an `auth::RequestInfo`: the client's IP address and user agent. They're used for rate limits, the audit trail, and the device's label in a list of sessions, such as "Firefox on macOS".

## Serve sign-in

With the `server` feature, `Auth` serves the whole flow over HTTP. Add its routes, and its layer last, so that it wraps every route:

```rust
let auth = auth::Auth::builder(&db, keys)
	.origin("https://myapp.com")
	.mailer(my_mailer)
	.on_sign_up::<User>()
	.build()?;

let app = my_app_backend::router()
	.merge(auth.router())
	.leptos_routes(&options, routes, move || shell(app_options.clone()))
	.layer(auth.layer());
```

The layer resolves each request's session before any handler runs. Handlers then ask for what they need:

```rust
async fn drafts(me: auth::SignedIn, Extension(db): Extension<Db>) -> Result<Json<Vec<PublicPost>>, StatusCode> {
	// `db` acts for this request: its policies see `me`
	…
}
```

- `auth::SignedIn` answers `401` for a signed-out request, and `Option<auth::SignedIn>` accepts both.
- `Extension<Db>` is the database, acting for the request. Don't add your own `Extension(db)` as well.
- In a leptos server function, `auth::require_signed_in()?` and `auth::request_db()?` do the same.

The routes take and return JSON:

| Route                               | Body                  | Does                                             |
| ----------------------------------- | --------------------- | ------------------------------------------------ |
| `POST /auth/email/send`             | `{ "email" }`         | Emails a code.                                   |
| `POST /auth/email/verify`           | `{ "email", "code" }` | Signs in, and sets the session cookie.           |
| `GET /auth/session`                 |                       | The current session, or `null`.                  |
| `POST /auth/sign-out`               |                       | Ends the current session, and clears the cookie. |
| `GET /auth/sessions`                |                       | The person's signed-in devices.                  |
| `POST /auth/sessions/revoke`        | `{ "id" }`            | Ends one of them.                                |
| `POST /auth/sessions/revoke-others` |                       | Ends all but the current one.                    |

A failure answers with a status and `{ "error", "message" }`. `error` is a code to match on, such as `invalid_code` or `rate_limited`, and a rate-limited answer also says `retry_after`. The server's own failures, such as a database outage, say only that something went wrong.

**On the web,** the session lives in a cookie that scripts can't read: `HttpOnly`, `SameSite=Lax`, and over HTTPS, a `__Host-` cookie that only your exact origin can set. On plain `http://localhost` in development it is a plain `session` cookie, since browsers refuse `Secure` cookies there.

**In a native app,** send `"transport": "bearer"` with the code. The token comes back in the body instead, for the app to keep in its secure store and send as `Authorization: Bearer …`.

**Requests from other sites** that would change something are refused with `403`. A browser's `POST` must carry your origin in `Origin`, or `Sec-Fetch-Site: same-origin`. A request that has neither is allowed only if it has no session cookie, which is how native apps and scripts call. Set `origin` to your site's URL; without it, a request must come from the host it was sent to.

The client's IP address, for rate limits and the audit trail, is the connection's peer address. Behind a proxy, name the header it sets, such as `.client_ip_header("fly-client-ip")`. Only name a header your proxy always overwrites, since anyone can send one.

### In development

Two builder options make sign-in easy to try. Release builds don't have either, so they can't ship by accident:

- `.dev_mailbox()` keeps every email instead of sending it, and shows them at `/_leptoskit/mail`.
- `.dev_sign_in()` adds `POST /auth/dev/sign-in`, which takes `{ "email" }` and signs in to that address without a code, creating its account if needed.

## Sessions

A session token is 256 random bits. The device keeps it, and the database keeps only its SHA-256 hash, so a copy of the database signs no one in. The layer turns the token back into the person on each request; outside a request, `auth.resolve(&token)` does the same.

A session ends after 14 days unused, or after 90 days however often it's used; `SessionLifetimes` changes both. A token that is unknown, expired, or revoked, or whose account is disabled, is signed out.

To end sessions from server code:

- `auth.sign_out(&token, &request)` ends the current session.
- `auth.revoke(&signed_in, session_id, &request)` ends one of the person's other sessions, such as a lost phone's. It never touches anyone else's.
- `auth.revoke_others(&signed_in, &request)` ends every session but the current one.
- `auth.rotate(&signed_in, &request)` replaces the current token with a new one, so a token captured earlier stops working. Rotate whenever the person's privileges change.
- `auth.disable(account, &request)` stops an account from signing in and ends every session it has. `auth.enable(account)` lets it sign in again.

## Rules about the signed-in person

The layer gives each request a database handle that knows who it's for. Outside the layer, build one yourself:

```rust
let db = db.with_context(auth::context(signed_in));
```

Policies read the signed-in person through `cx.signed_in`, which matches nothing for a signed-out request:

```rust
use leptoskit::auth::AuthContext;

#[db::table(name = "posts")]
#[db::policy(all, |post, cx| cx.signed_in(|me| post.author_id.eq(me.user_id())))]
pub struct Post {
	pub id: Id<Post>,
	pub author_id: Id<User>,
	pub body: String,
}
```

`me.user_id()` is the person's ID in your user table, since it shares the account's key.

The auth tables have policies of their own. A signed-in person reads only their own account, emails, sessions, and events, and nobody reads codes or rate-limit counters. Only `leptoskit_auth` writes any of them.

## Limits

Every step is rate limited, by address and by IP address:

| Step         | Default limit                                    |
| ------------ | ------------------------------------------------ |
| Sending code | 5 per address per 15 minutes; 20 per IP per hour |
| Checking one | 30 per IP per 15 minutes, and 5 guesses per code |

Change them with `Limits`. An attempt over the limit fails with `AuthError::RateLimited`, which says how long to wait. The counters live in `auth.rate_limits`, and concurrent attempts can't both take the last one. To count somewhere faster, such as Redis, implement `auth::RateLimiter`.

## Test sign-in

`auth::Mailbox` keeps the emails a test sends, so a test signs in the way a person would:

```rust
let mailbox = auth::Mailbox::new();
let auth = auth::Auth::builder(&db, auth::Keys::single("test", [7; 32]))
	.mailer(mailbox.clone())
	.on_sign_up::<User>()
	.build()?;

auth.send_email_code("ada@example.com", &auth::RequestInfo::default()).await?;
let mail = mailbox.latest_to("ada@example.com").unwrap();
let code = mail.subject.rsplit(' ').next().unwrap();
let sign_in = auth.verify_email_code("ada@example.com", code, &auth::RequestInfo::default()).await?;
```
