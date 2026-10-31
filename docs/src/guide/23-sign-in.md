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

The app opens on a sign-in screen. Type any address, then open `/_leptoskit/mail` to read the code, which a debug build keeps instead of sending. Once you're signed in, the bar at the top lists where you're signed in, opens the security screen for [passkeys](#passkeys), [two-step verification](#two-step-verification), and [connected accounts](#sign-in-with-other-sites), and signs you out. The project has a SQLite database unless `--db postgres` picks Postgres, a `User` table that gets a row for each new account, and the server's routes and layer.

For an app you already have, generate the screens:

```sh
leptoskit generate auth
```

It writes three screens into your app's `src/auth/` folder: a sign-in screen, with an emailed code, a passkey button and the email field's passkey suggestions, a "Continue with …" button for each provider, and the second step when an account has one; a "where you're signed in" screen; and a security screen for passkeys, an authenticator app with its QR code, connected accounts, and merging accounts. It also adds what they need:

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

The newest key hashes new codes and encrypts new authenticator secrets, and any listed key checks or decrypts one. To rotate, put a new key first, keep the old one listed, and run `leptoskit auth rotate-keys` to re-encrypt what's stored. Recovery codes hashed with a removed key stop working, so ask people to make new ones before you remove it. Keep the variable in your secret store.

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

It's optional unless you [require it](#require-it), and each person turns it on for themselves. The generated security screen does all of it; this section explains what that screen calls.

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

1. `enroll_totp()` makes a new secret. Show its `uri` as a QR code for the phone's camera, and its `secret`, grouped for typing. `auth::client::qr_code(&setup.uri)` makes the code on the device, so the secret never goes to another service; draw its `path()` in a `Vector` (every platform but HarmonyOS draws one), or use its `svg()` on the web.
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

### Require it

`mfa` in `[auth]` decides who must use two-step verification:

```toml
[auth]
mfa = "required" # everyone; the default is "optional"
required_for = ["Admin"] # or only people with these roles
```

For `required_for`, tell sign-in how to read an account's roles:

```rust
impl auth::Roles for User {
	async fn roles(db: &db::Db, account: db::Id<auth::Account>) -> auth::Result<Vec<String>> {
		let user = User::get(db, account.into()).await?;
		Ok(vec![user.role.to_string()])
	}
}

let auth = auth::Auth::from_config(&db, &config)?
	.on_sign_up::<User>()
	.roles::<User>()
	.build()?;
```

Someone who must use it, and hasn't, still signs in with their emailed code, but their session can only set up a passkey or an authenticator app, read its own status, and sign out. Everything else answers `mfa_required`, and your policies see no one. `use_mfa()` reports `required: true`, so the generated screens send them to the security screen; once a factor is on, the session works normally. The last passkey or authenticator of an account that must have one can't be removed (`last_sign_in_method`).

### Locked accounts

After 20 wrong second factors in a day (authenticator codes, recovery codes, or passkeys, at sign-in or step-up), the account is locked for an hour, and its owner is emailed: getting that far needed their emailed code, or a device they're signed in on. Signing in answers `locked` until then. Wrong emailed codes don't count, since anyone who knows an address can get those wrong. `Limits::failures_before_lock` and `Limits::lock_for` change this, and `auth.unlock(account, &request)` lifts a lock early.

When someone is locked out of their own two-step verification, such as after losing both their phone and their recovery codes, an administrator can check who they are another way and call `auth.reset_mfa(account, &request)`. It removes the account's authenticator, recovery codes, and passkeys, lifts any lock, signs out every device, and emails the account. The emailed code alone then signs them in.

## Passkeys

An emailed code can be phished: a fake site asks for it and passes it on. A passkey can't. It's a key pair the person's device makes for your site alone, unlocked with their fingerprint, face, or screen lock, and the browser only lets your origin use it. It's also faster than a code: one tap.

Passkeys need the `auth-passkeys` feature on your server's `leptoskit` dependency, and `[auth] origin`: its host is the relying party, so `https://myapp.com` makes passkeys that work on `myapp.com` and nowhere else. The feature builds OpenSSL from source the first time, which adds a minute or two to a clean build. Without it, the functions below still exist, and the server answers `unsupported`.

### Add one

From the security screen, after a recent sign-in (or step-up, once the account has a second factor):

```rust
use leptoskit::auth::client::add_passkey;

let add = use_mutation(add_passkey());
add.mutate(Some("Work laptop".into()));        // PasskeyAdded { passkey, session, recovery_codes }
```

The browser asks the person to confirm with their fingerprint, face, or screen lock. The passkey is named as given, or after the device, such as "Passkey on Safari on macOS". Like turning on an authenticator, adding one emails the account, moves the session to `aal2` with a new token, and gives an account without recovery codes ten of them, shown once.

### Sign in with one

```rust
use leptoskit::auth::client::PasskeyMode;
use leptoskit::auth::client::passkey_sign_in;

let passkey = use_mutation(passkey_sign_in());
passkey.mutate(PasskeyMode::Prompt);           // "Sign in with a passkey"
passkey.mutate(PasskeyMode::Autofill);         // offered in the email field's suggestions
```

No email address is needed: the device offers the person's passkeys for your site. `Autofill` waits for them to pick one from the email field's suggestions, so start it when the screen opens, and give the field `.autocomplete("username webauthn")`. `cancel_passkey_sign_in()` stops it when they choose another way. A passkey that checks who's holding the device is two factors at once, so passkey sign-in starts an `aal2` session, with no second step.

Once an account has a passkey, signing in with an emailed code asks for it as the second step, with `Factor::Passkey` first in the prompt's factors. `complete_sign_in_with_passkey()` finishes, and `step_up_with_passkey()` answers `step_up_required`. Recovery codes stand in for a lost passkey, as they do for a lost phone.

### Manage them

`use_passkeys()` lists them, newest first: each one's name, whether it's synced to the person's other devices, when it was added, and when it was last used. `rename_passkey()` renames one; `remove_passkey()` removes one after step-up, and signs out every other device.

### On every platform

| Platform                                                               | Passkeys                                                                            |
| ---------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| Web                                                                    | Everything above; `Autofill` where the browser offers passkeys in suggestions       |
| iOS, Android, macOS, Windows, Linux, and HarmonyOS (experimental) apps | The same functions answer `unsupported`; listing, renaming, and removing still work |

Native passkeys need the platforms' own passkey APIs, which arrive with a plugin; until then, native apps sign in with an emailed code and an authenticator app. Your code doesn't change when they arrive.

### How it's kept safe

- The server stores only public keys. A copy of the database signs no one in.
- Each ceremony's challenge is kept on the server, encrypted, for five minutes, and works once, even when the answer is wrong. The browser holds only a random token in an `HttpOnly` cookie.
- Passkeys must verify their user, and a signature counter that goes backwards, which can mean a cloned key, is refused.
- A passkey from another site, or registered to another account, is refused, and the attempt recorded.

## Sign in with other sites

Many people would rather "Continue with Google" than wait for an email. leptoskit signs in with Google, Apple, Microsoft, and GitHub, and with any OpenID Connect provider, such as Okta, Auth0, Keycloak, or your company's Entra tenant. Each has a [setup page](auth/providers/google.md).

Add one from the command line, which writes its `[auth.providers.<id>]` table and prints its setup steps:

```sh
leptoskit auth provider add google
leptoskit auth provider add company --issuer https://sso.example.com
```

```toml
[auth.providers.google]
client_id_env = "GOOGLE_CLIENT_ID"
client_secret_env = "GOOGLE_CLIENT_SECRET"

[auth.providers.company]
name = "Company SSO"
issuer = "https://sso.example.com"
client_id_env = "COMPANY_CLIENT_ID"
client_secret_env = "COMPANY_CLIENT_SECRET"
```

Then turn on the `auth-providers` feature on your server's `leptoskit` dependency. `Auth::from_config` reads each provider's secrets from the variables it names; `AuthBuilder::provider(auth::Provider::google(…))` does the same in code.

### Signing in

```rust
use leptoskit::auth::StartProvider;
use leptoskit::auth::client::provider_outcome;
use leptoskit::auth::client::sign_in_with_provider;
use leptoskit::auth::client::use_providers;

let providers = use_providers();               // [ProviderInfo { id: "google", name: "Google" }, …]
let start = use_mutation(sign_in_with_provider());
start.mutate(StartProvider { provider: "google".into(), redirect_to: Some("/notes".into()) });
```

The page goes to the provider's sign-in page, and comes back to `redirect_to` (a path on your site; anything else is refused) with the outcome in its query, which `provider_outcome()` reads:

| Outcome            | Meaning                                                                                   |
| ------------------ | ----------------------------------------------------------------------------------------- |
| `SignedIn`         | Signed in, or signed up                                                                   |
| `SecondFactor(..)` | The account has two-step verification; finish with a code or a passkey, as after an email |
| `EmailInUse`       | Another account already has the provider's email address; see below                       |
| `Linked`           | Connected to the signed-in account                                                        |
| `AlreadyLinked`    | It was connected already                                                                  |
| `Merge`            | It belongs to another account here, which can be merged into this one                     |
| `Failed(code)`     | It didn't work: `cancelled`, `sign_in_expired`, `provider`, `locked`, …                   |

Who someone is at a provider is the provider's own ID for them, never their email address: some providers let people set any address, and trusting it is how accounts get taken over. So a provider sign-in never signs in to an account just because the email matches. When it does match, the outcome is `EmailInUse`: the person can sign in to that account with their email and connect the provider from its security screen, or `continue_provider_sign_up()` for a separate account without that address. An address is only stored as verified when the provider vouches for it; Microsoft's never is.

### Connecting, disconnecting, and merging

From the security screen, `link_provider()` connects a provider to the signed-in account, after a recent sign-in or step-up. `use_identities()` lists connected accounts, and `unlink_identity()` disconnects one, as long as the account keeps a way to sign in.

If the provider's account already has its own account here, say from signing up with GitHub months ago, the outcome is `Merge`. `use_merge_offer()` describes the other account, with a preview of its data, and `merge_accounts()` merges it after the person confirms, with `confirm: true`:

```rust
use leptoskit::auth::MergeMode;
use leptoskit::auth::client::MergeInput;
use leptoskit::auth::client::merge_accounts;

let merge = use_mutation(merge_accounts());
merge.mutate(MergeInput { mode: Some(MergeMode::BringData), confirm: true, code: Some(code_from_its_authenticator), ..MergeInput::default() });
```

Merging moves the other account's email addresses, connected accounts, passkeys, and wallets here, and either brings its data along, as your tables' `on_account_merge` strategies say, or deletes it. It can't be undone, and needs a recent sign-in or step-up on this account and the other account's second factor when it has one. [Merging accounts](auth/merging.md) covers both modes, the preview, and your own merge code.

### On every platform

| Platform                                                               | Third-party sign-in                                                                                                                  |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| Web                                                                    | Everything above                                                                                                                     |
| iOS, Android, macOS, Windows, Linux, and HarmonyOS (experimental) apps | `sign_in_with_provider()`, `link_provider()`, and `continue_provider_sign_up()` answer `unsupported`; listing and disconnecting work |

Native apps need the system browser to sign in with another site safely, which arrives with a plugin; your code doesn't change when it does.

### How it's kept safe

- Every flow uses PKCE (`S256`), a random `state`, and, for OpenID Connect, a `nonce`. They're kept on the server, encrypted, for ten minutes, and the browser keeps only a random token in an `HttpOnly` cookie, so a sign-in only finishes in the browser that started it.
- ID tokens must be signed by a key the provider publishes (RS256, PS256, ES256, ES384, or EdDSA; never `none` or a shared secret), for your client, from the right issuer (for Microsoft, the person's own tenant), unexpired, and with the flow's nonce.
- Apple posts its answer from its own site, which `SameSite` cookies don't follow, so that post only gets a page that posts it again from your site; it never signs anyone in by itself.
- Provider secrets come from environment variables; Apple's client secret is signed for each request from your `.p8` key, so there's nothing to rotate.

## Solana wallets

People can link a Solana wallet to their account with a message their wallet signs for your site, show it on their profile, require a fresh signature before a payout, and, if they choose, sign in with it. Rules can name a wallet's owner with `cx.wallets()`. In the browser, any wallet that supports the Wallet Standard works; native apps manage linked wallets, and answer `unsupported` for asking a wallet to sign until a native wallet adapter arrives. See [Solana wallets](auth/wallets.md).

## Check the configuration

`leptoskit auth doctor` checks `[auth]` against the environment it runs in, and never prints a secret:

- that each variable is set, and the keys parse;
- the origin, and the relying party passkeys belong to;
- where each provider sends people back, and whether its built-in definition was checked within the last year;
- that an iOS app with third-party sign-in also offers Sign in with Apple, as the App Store expects.

With `--release`, it fails on what a release build mustn't have: a plain `http` origin, the development mailbox, or a plain `http` origin in `[native.plugin-options.http]` origins, which would send a native app's tokens unencrypted. Run it in CI before you ship.

`leptoskit auth rotate-keys` re-encrypts stored authenticator secrets, and sign-ins in progress, under the newest key in `AUTH_ENCRYPTION_KEYS`, so you can retire an old key. It's safe to run while the app is running, and again. Recovery codes and emailed codes are hashes, which can't be re-encrypted: keep the old key listed until people have made new recovery codes, or accept that their old ones stop working.

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

`cx.wallets()` is the addresses of the person's [linked wallets](auth/wallets.md), so a rule can say "whoever holds this wallet". It's empty for a signed-out request, so the rule matches nothing then:

```rust
#[db::table(name = "vaults")]
#[db::policy(update, |vault, cx| vault.treasury.in_list(cx.wallets()))]
pub struct Vault {
	pub id: Id<Vault>,
	pub treasury: Pubkey,
}
```

`auth.request_context(signed_in)` builds a context with both, as the layer does; `auth::context(signed_in)` has the person without their wallets.

The auth tables have policies of their own. A signed-in person reads only their own account, emails, sessions, and events, and nobody reads codes or rate-limit counters. Only `leptoskit_auth` writes any of them.

## Limits

Every step is rate limited, by address and by IP address:

| Step                                            | Default limit                                        |
| ----------------------------------------------- | ---------------------------------------------------- |
| Sending code                                    | 5 per address per 15 minutes; 20 per IP per hour     |
| Checking one                                    | 30 per IP per 15 minutes, and 5 guesses per code     |
| Authenticator or recovery code                  | 10 per account per 15 minutes, and 5 per second step |
| Starting a passkey, provider, or wallet request | 120 per IP per 15 minutes                            |
| Signing in with a wallet                        | 30 per IP per 15 minutes                             |
| Wrong second factors                            | 20 per account per day, then locked for an hour      |

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
