# Sign-in

Sign-in is the part of an app where small mistakes are the most expensive. Session tokens get stored as they are, so a leaked database backup signs everyone in. A form says "no account with that email", which tells an attacker whose accounts exist. A code can be guessed because nothing counts the guesses. And one server function forgets to check who is asking.

`leptoskit::auth` handles all of these in one place, in your app's own database. It keeps only hashes of tokens and codes, gives the same answer for every address, and limits every attempt. It also tells your access policies who is signed in, so each table's rules apply on every query.

## Start from a template

Wiring sign-in by hand means a config table, a user table, server routes, and two screens, before anyone can sign in at all. leptoskit writes all of it for you, and the rest of this page explains each piece.

For a new app, pass `--auth`:

```sh
leptoskit new my_app --auth
cd my_app
leptoskit db migration create init
export AUTH_ENCRYPTION_KEYS="$(date +%Y-%m):$(openssl rand 32 | basenc --base64url | tr -d =)"
leptoskit dev
```

The app opens on a sign-in screen. Type any address, then open `/_leptoskit/mail` to read the code, which a debug build keeps instead of sending. Once you're signed in, the bar at the top lists where you're signed in, turns on [two-step verification](#two-step-verification), and signs you out. The project has a SQLite database unless `--db postgres` picks Postgres, a `User` table that gets a row for each new account, and the server's routes and layer.

For an app you already have, generate the screens:

```sh
leptoskit generate auth
```

It writes three screens into your app's `src/auth/` folder: an email-and-code sign-in screen, which also asks for the second step when an account has one; a "where you're signed in" screen; and a two-step verification screen. It also adds what they need:

- the `[auth]` table in `leptoskit.toml`, and the `auth`, `leptos`, and `query` features of `leptoskit`;
- `/sign-in`, `/devices`, and `/security` routes, if your app uses the [file router](10-file-routes.md);
- a `src/sign_in.rs` module that builds the service, if your app has a server and a database;
- for an app with iOS, Android, or macOS hosts, the `http` and `secure-store` plugins those hosts need, granted in `leptoskit.toml` and added to each host's `Cargo.toml`.

Then it lists the few lines it can't add for you, such as putting the sign-in screen in front of your app. Running it again changes nothing, and a screen you've edited is kept unless you pass `--force`.

The screens are built from leptoskit's own components and Tailwind classes, on the [sign-in client](#build-the-sign-in-screen), so the same files build and render in the browser and in native apps on every platform. They show a message while they load, show the server's error messages, and after too many attempts disable the button until the server will take another. They're yours to restyle.

In a native app the screens reach the server through the `http` plugin and keep the session with `secure-store`. A project made with `--auth` grants both, lets the HTTP plugin reach the development server, and looks for its server at the `SERVER` address in `my_app_app/src/lib.rs`, `http://localhost:3000` until you change it. Before you ship, set `SERVER` to your site and list it in the HTTP plugin's `origins`, as [Build the sign-in screen](#build-the-sign-in-screen) shows.

What happens on each platform:

- **Web:** signing in works as soon as the server runs.
- **iOS, Android, and macOS:** the generated apps register both plugins, so signing in works once the app can reach `SERVER`.
- **Windows, Linux, and the terminal:** these runners can't host plugins yet. They build without them, `leptoskit build` says so, and the screens show that sign-in isn't set up: every client call answers the error code `config`.

## Add the auth tables

Sign-in keeps its tables in your database as a [schema pack](22-database.md#tables-that-come-from-a-library). Turn on the `auth` feature next to your database feature, then add the pack in the crate that declares your tables:

```toml
leptoskit = { version = "…", features = ["auth", "db-postgres"] }
```

```rust
leptoskit::db::packs!(leptoskit_auth);
```

Your next `leptoskit db migration create` adds the tables, in the `auth` namespace:

| Table            | What it holds                                                                    |
| ---------------- | -------------------------------------------------------------------------------- |
| `accounts`       | One row per person: when they joined, and whether the account is disabled.       |
| `emails`         | The addresses each account has proved it receives mail at.                       |
| `sessions`       | Signed-in devices: hashes of their tokens, their lifetimes, and a label.         |
| `challenges`     | Codes in flight, as keyed hashes. They expire and work once.                     |
| `events`         | The audit trail: sign-ups, sign-ins, failures, and revocations, with IP address. |
| `rate_limits`    | Attempt counters, keyed by a hash, so the table holds no addresses.              |
| `totp`           | Each account's authenticator app secret, encrypted with the server's keys.       |
| `recovery_codes` | Each account's recovery codes, as keyed hashes. Each works once.                 |

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

Describe sign-in in `leptoskit.toml`. Secrets never go in the file; each setting that needs one names the environment variable that holds it:

```toml
[auth]
origin = "https://myapp.com"
keys_env = "AUTH_ENCRYPTION_KEYS" # the default
idle_days = 14 # the default
absolute_days = 90 # the default
access_minutes = 15 # the default: how long a native app's access token works

[auth.mail]
from = "My App <hello@myapp.com>"
smtp_url_env = "SMTP_URL"
```

Build one `Auth` from it when the server starts, add your sign-up hook, and share it:

```rust
// Embedded, so the server needs no file at runtime.
let config = leptoskit::config::LeptoskitConfig::from_toml(
	include_str!("../../leptoskit.toml"),
	std::path::Path::new("my_app"),
)?;
let auth = auth::Auth::from_config(&db, &config)?
	.on_sign_up::<User>()
	.build()?;
```

`AUTH_ENCRYPTION_KEYS` holds the server's keys, newest first, as `id:key` pairs, where each key is 32 random bytes in base64url:

```sh
echo "AUTH_ENCRYPTION_KEYS=$(date +%Y-%m):$(openssl rand 32 | basenc --base64url | tr -d =)"
```

The newest key hashes new codes and encrypts new authenticator secrets, and any listed key checks or decrypts one. To rotate, put a new key first and keep the old one listed: until the codes it hashed have expired, and for as long as authenticator secrets encrypted with it are in use. Recovery codes hashed with a removed key stop working, so ask people to make new ones before you remove it. Keep the variable in your secret store.

`Auth::builder(&db, keys)` configures the same things in code, for tests and for apps without a `leptoskit.toml`.

### Send email

`[auth.mail]` picks one way to send:

- **SMTP** (the `auth-smtp` feature): `smtp_url_env` names a variable holding a URL such as `smtps://user:password@smtp.example.com`, and `from` is who email comes from.
- **A webhook** (the `auth-webhook` feature): `webhook_url_env` and `webhook_secret_env` name a URL you control and its signing secret. Each email arrives there as JSON, `{ "type": "email", "from", "to", "subject", "text" }`, signed the way [Standard Webhooks](https://www.standardwebhooks.com) describes, and your endpoint sends it through any provider. Check the `webhook-signature` header before you trust a request.

With neither, a debug build keeps every email in the development mailbox, and a release build refuses to start rather than silently send nothing. Any other provider takes a few lines: implement `auth::Mailer`, and pass it to `.mailer(…)`.

## Sign in with an emailed code

One flow both signs in and signs up. Ask for a code, and the person receives one by email:

```rust
auth.send_email_code(&address, &request).await?;
```

The answer is the same for every valid address, whether or not it has an account, so the form can't be used to find out who has signed up. When the person types the code, check it:

```rust
match auth.verify_email_code(&address, &code, &request).await? {
	auth::SignInStep::SignedIn(sign_in) => {
		// sign_in.token goes to the device; sign_in.signed_up says whether the account is new
	}
	auth::SignInStep::SecondFactor(step) => {
		// the account has two-step verification: see below
	}
}
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

| Route                               | Body                  | Does                                                        |
| ----------------------------------- | --------------------- | ----------------------------------------------------------- |
| `POST /auth/email/send`             | `{ "email" }`         | Emails a code.                                              |
| `POST /auth/email/verify`           | `{ "email", "code" }` | Signs in, and sets the session cookie.                      |
| `POST /auth/refresh`                | `{ "refresh_token" }` | Trades a native app's refresh token for new tokens.         |
| `GET /auth/session`                 |                       | The current session, or `null`.                             |
| `POST /auth/sign-out`               |                       | Ends the current session, and clears the cookie.            |
| `GET /auth/sessions`                |                       | The person's signed-in devices, this one marked `current`.  |
| `POST /auth/sessions/revoke`        | `{ "id" }`            | Ends one of them.                                           |
| `POST /auth/sessions/revoke-others` |                       | Ends all but the current one.                               |
| `POST /auth/sessions/revoke-all`    |                       | Ends every one, this one included, and clears the cookie.   |
| `POST /auth/second-factor`          | `{ "code" }`          | Finishes signing in with an authenticator or recovery code. |
| `GET /auth/mfa`                     |                       | Whether an authenticator is on, and recovery codes left.    |
| `POST /auth/step-up`                | `{ "code" }`          | Confirms it's still the signed-in person.                   |
| `POST /auth/totp`                   |                       | Starts turning an authenticator app on.                     |
| `POST /auth/totp/confirm`           | `{ "code" }`          | Turns it on with a code from it.                            |
| `POST /auth/totp/disable`           |                       | Turns it off, after step-up.                                |
| `POST /auth/recovery-codes`         |                       | Replaces the recovery codes, after step-up.                 |

A failure answers with a status and `{ "error", "message" }`. `error` is a code to match on, such as `invalid_code` or `rate_limited`, and a rate-limited answer also says `retry_after`. The server's own failures, such as a database outage, say only that something went wrong.

**On the web,** the session lives in a cookie that scripts can't read: `HttpOnly`, `SameSite=Lax`, and over HTTPS, a `__Host-` cookie that only your exact origin can set. On plain `http://localhost` in development it is a plain `session` cookie, since browsers refuse `Secure` cookies there.

**In a native app** on iOS, Android, macOS, Windows, or Linux there are no cookies to lean on, so the app holds two tokens instead. The [sign-in client](#build-the-sign-in-screen) does all of the following for you; it matters only to code that calls the routes itself. Send `"transport": "bearer"` with the code, and optionally a `"device_name"` such as "Ada's iPhone" for the list of devices. The answer carries `tokens`:

```json
{ "access_token": "…", "refresh_token": "…", "expires_in": 900 }
```

- Send the **access token** with each request as `Authorization: Bearer …`. It works for 15 minutes; `access_minutes` changes that, up to an hour.
- Keep the **refresh token** in the platform's secure store, never in plain storage. Before the access token runs out, or after a `401`, trade it at `POST /auth/refresh` for a new pair. It works once: the old access token stops working at the same moment.
- Refresh one request at a time. If a refresh's response is lost, retrying with the same refresh token works for 30 seconds. After that, the server takes a second use as a sign that someone copied it, ends the session for every holder, and records a `refresh_reused` event; the app has to sign in again.

**Requests from other sites** that would change something are refused with `403`. A browser's `POST` must carry your origin in `Origin`, or `Sec-Fetch-Site: same-origin`. A request that has neither is allowed only if it has no session cookie, which is how native apps and scripts call. A request with `Authorization: Bearer` and no session cookie is allowed from any origin, because browsers never send that header on their own. Another origin may also ask for a code and sign in with `"transport": "bearer"`, which sets no cookie; only signing in for a cookie must come from your own site. That is how a [browser extension](../platforms/extension.md) signs in and calls from its own origin. Set `origin` to your site's URL; without it, a request must come from the host it was sent to.

The client's IP address, for rate limits and the audit trail, is the connection's peer address. Behind a proxy, name the header it sets, such as `.client_ip_header("fly-client-ip")`. Only name a header your proxy always overwrites, since anyone can send one.

### In development

Two builder options make sign-in easy to try. Release builds don't have either, so they can't ship by accident:

- `.dev_mailbox()` keeps every email instead of sending it, and shows them at `/_leptoskit/mail`.
- `.dev_sign_in()` adds `POST /auth/dev/sign-in`, which takes `{ "email" }` and signs in to that address without a code, creating its account if needed.

## Build the sign-in screen

The sign-in screen is yours to design; `leptoskit::auth::client` supplies the behaviour, as [query and mutation scopes](11-fetching-data.md). They are the same functions, taking the same arguments, on the web and in native apps on iOS, Android, macOS, Windows, and Linux. One screen works everywhere, and your code never asks which platform it's on.

First, tell the client where your server is. Call `configure` once, when the app starts, on every platform:

```rust
use leptoskit::auth::client::ClientConfig;
use leptoskit::auth::client::configure;

configure(
	ClientConfig::new()
		.origin("https://myapp.com")                 // where the server is
		.device_name("Ada's iPhone"),                // optional: this device in the device list
);
```

- **On the web**, the origin can be left out: the client talks to the page's own origin. If you set it, it must be that origin, because the session cookie belongs to it. Browsers are named in the device list from their user agent, so the device name is ignored.
- **In a native app**, the origin is required. Without it, every call fails with the error code `config`. The app also needs the `http` and `secure-store` [plugins](../platforms/plugins.md), with the same origin in the HTTP plugin's `origins`. `leptoskit new --auth` and `leptoskit generate auth` grant both; add your site's origin before you ship:

  ```toml
  [native]
  plugins = ["http", "secure-store"]

  [native.plugin-options.http]
  origins = ["https://myapp.com", "http://localhost:3000"]
  ```

  The generated iOS, Android, and macOS hosts register both plugins. The Linux, Windows, and terminal runners can't host plugins yet: they build without them, with a warning, and the client answers `config` there. An app with its own Windows or Linux host can register them by hand; see the [plugin support matrix](../platforms/plugin-inventory.md#storage-and-networking).

Then build the screen:

```rust
use leptoskit::auth::client::EmailCode;
use leptoskit::auth::client::send_code;
use leptoskit::auth::client::sign_out_mutation;
use leptoskit::auth::client::use_auth;
use leptoskit::auth::client::verify_code;
use leptoskit::query::use_mutation;

let auth = use_auth();                          // the session query
let send = use_mutation(send_code());
let verify = use_mutation(verify_code());
let sign_out = use_mutation(sign_out_mutation());

send.mutate(email.clone());                     // step 1: email a code
verify.mutate(EmailCode { email, code });       // step 2: sign in, or SignInStep::SecondFactor
```

- `auth.data()` is `Some(Some(session))` when signed in, and `Some(None)` when signed out.
- Signing in and out update that query, so everything that reads `use_auth()` re-renders, with no refetch.
- A failed step's `error()` is an `ErrorResponse`. Show its `message`, or match on its `error` code: for example, after `rate_limited`, disable the button for `retry_after` seconds. `network` means the server couldn't be reached.

What happens underneath depends on the platform, and none of it is yours to handle:

- **In the browser**, the session lives in the `HttpOnly` cookie, which scripts can't read, including any injected into your page. The client keeps nothing itself; each call is a request to your own origin that carries the cookie.
- **While the server renders a page**, `use_auth()` and `use_devices()` read the request's own session directly, so the first page already shows who is signed in. Signing in and out happen in the browser once the page loads.
- **In a native app**, the client keeps the short-lived access token in memory, and the refresh token in the platform's secure store, such as the Keychain on Apple platforms, so the person stays signed in when the app restarts. When the access token runs out, the client trades the refresh token for new ones and repeats the request, one trade at a time. If the session has ended, for example because the person signed out everywhere from another device, the client forgets its tokens, and `use_auth()` reports signed out. Signing out forgets the tokens even when the server can't be reached.

### Show where they're signed in

A "where you're signed in" screen lists each device, and lets the person end any of them: a lost phone, a shared computer, or everything at once after a scare. It works the same on every platform.

```rust
use leptoskit::auth::client::revoke_device;
use leptoskit::auth::client::revoke_other_devices;
use leptoskit::auth::client::sign_out_everywhere_mutation;
use leptoskit::auth::client::use_devices;

let devices = use_devices();                    // Vec<Device>, most recently used first
let revoke = use_mutation(revoke_device());
let others = use_mutation(revoke_other_devices());
let everywhere = use_mutation(sign_out_everywhere_mutation());

revoke.mutate(device.session.id);               // end one device; `true` if it was still signed in
others.mutate(());                              // end every device but this one
everywhere.mutate(());                          // end them all, this one included
```

Each `Device` has the session's `device` label, when it signed in, when it was last used, and `current`, which marks the device you're on. Ending a device ends its session at once: its next request is signed out, and a native app's refresh token stops working with it. Each mutation refreshes the list, and signing out everywhere also clears the session query.

## Two-step verification

An emailed code proves someone can read the inbox, so whoever gets into a person's email can also get into their account. Two-step verification closes that gap: the person turns on an authenticator app, such as 1Password, Google Authenticator, or the iPhone's Passwords app, and from then on signing in asks for the six-digit code it shows after the emailed one.

It's optional, and each person turns it on for themselves. The generated two-step verification screen does all of it; this section explains what that screen calls.

### Turn it on

```rust
use leptoskit::auth::client::confirm_totp;
use leptoskit::auth::client::enroll_totp;
use leptoskit::auth::client::use_mfa;

let status = use_mfa();                         // MfaStatus { totp, recovery_codes }
let enroll = use_mutation(enroll_totp());
let confirm = use_mutation(confirm_totp());

enroll.mutate(());                              // TotpSetup { secret, uri }
confirm.mutate(code_from_the_app);              // TotpConfirmation { session, recovery_codes }
```

1. `enroll_totp()` makes a new secret. Show the person its `secret`, grouped for typing, or its `otpauth://` `uri`, which apps that take a link accept.
2. They add it to their app, and type the code it shows. `confirm_totp()` turns the authenticator on only if that code matches, so a mistyped secret never locks anyone out.
3. The first time, they also get ten **recovery codes**, such as `K7QMR-2XDPA`. Each signs in once, in place of the app, if the phone is lost. Show them once and ask the person to save them; the server keeps only their hashes.

Turning it on also emails the account, so the owner hears about it if it wasn't them.

Starting needs a recent sign-in, within ten minutes, so a session left open on a shared computer can't turn on an authenticator that locks the owner out. Otherwise it answers `reauthenticate`, and the screen asks the person to sign in again.

### Signing in with it

Once it's on, `verify_code()` answers `SignInStep::SecondFactor` instead of signing in. Finish with the code from the app, or a recovery code:

```rust
use leptoskit::auth::client::SignInStep;
use leptoskit::auth::client::complete_sign_in;

let finish = use_mutation(complete_sign_in());

// after verify answered SignInStep::SecondFactor(prompt):
finish.mutate(code);                            // the session query now holds the session
```

- The second step lasts five minutes and allows five attempts; then it answers `sign_in_expired`, and the person starts again from the email.
- An authenticator code works once, even within its 30 seconds, and codes from a phone whose clock is up to 30 seconds off still work.
- Using a recovery code signs out every other device and emails the account, since it usually means a lost phone.
- In the browser, the step is held in an `HttpOnly` cookie for its five minutes; in a native app, the client holds it in memory. Either way your code only passes the code.

### Sensitive actions: step-up

Some actions should need more than a session that was opened days ago: changing how someone signs in, deleting the account, or moving money. Guard them with step-up, which asks for a code from the authenticator again:

```rust
#[server]
pub async fn delete_account() -> Result<(), ServerFnError> {
	let me = auth::require_step_up(auth::STEP_UP_WINDOW)?;   // a code within the last ten minutes
	// …
	Ok(())
}
```

Without a recent code it fails with `step_up_required`. The screen then asks for a code, calls `step_up()`, and tries the action again:

```rust
use leptoskit::auth::client::step_up;

let confirm = use_mutation(step_up());
confirm.mutate(code);                           // then retry what asked for it
```

Step-up gives the session a new token, so one copied before stops working. In handlers outside server functions, `signed_in.require_step_up(auth::STEP_UP_WINDOW)?` does the same check.

### Turn it off, or make new codes

`disable_totp()` turns the authenticator off and deletes the recovery codes; `regenerate_recovery_codes()` replaces the codes, and the old ones stop working. Both need step-up first. Turning it off also signs out every other device and emails the account.

### On every platform

Authenticator codes and recovery codes work the same in the browser and in native apps: the same functions, the same screens, and the same error codes. The only difference is where the session lives, which the client handles.

### How it's kept safe

- The authenticator's secret is encrypted in the database with AES-256-GCM-SIV under your `AUTH_ENCRYPTION_KEYS`, and bound to its account, so a copy of the database can't produce codes.
- Recovery codes are stored as keyed hashes, and every code is compared in constant time.
- Attempts are limited per account and per IP address, on top of the second step's own five.
- Every change, failed code, and recovery code use is recorded in the audit trail, and the account is emailed about changes.

## Sessions

Every token is 256 random bits. The device keeps it, and the database keeps only its SHA-256 hash, so a copy of the database signs no one in. Tokens aren't signed, and carry nothing a script could read or forge: the server looks each one up on every request, so ending a session takes effect immediately, with no lifetime to wait out. The layer turns the token back into the person on each request; outside a request, `auth.resolve(&token)` does the same.

A session ends after 14 days unused, or after 90 days however often it's used; `SessionLifetimes` changes both, and how long a native app's access token works. A token that is unknown, expired, or revoked, or whose account is disabled, is signed out.

To end sessions from server code:

- `auth.sign_out(&token, &request)` ends the current session.
- `auth.revoke(&signed_in, session_id, &request)` ends one of the person's other sessions, such as a lost phone's. It never touches anyone else's.
- `auth.revoke_others(&signed_in, &request)` ends every session but the current one.
- `auth.sign_out_everywhere(account, &request)` ends every session an account has, the current one included. Use it for a "sign out everywhere" button, or when an account is stolen.
- `auth.devices(&signed_in)` lists the person's live sessions, the current one marked.
- `auth.refresh(&refresh_token, &request)` trades a native app's refresh token, as `POST /auth/refresh` does.
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

| Step                           | Default limit                                        |
| ------------------------------ | ---------------------------------------------------- |
| Sending code                   | 5 per address per 15 minutes; 20 per IP per hour     |
| Checking one                   | 30 per IP per 15 minutes, and 5 guesses per code     |
| Authenticator or recovery code | 10 per account per 15 minutes, and 5 per second step |

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
let sign_in = auth
	.verify_email_code("ada@example.com", code, &auth::RequestInfo::default())
	.await?
	.signed_in()
	.expect("no second factor yet");
```
