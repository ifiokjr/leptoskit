# Sign in with your app

Once people have accounts in your app, other apps start asking for them: a partner's site wants a "Sign in with Notes" button, your own command-line tool wants to act for its user, or someone builds an integration on your API. Handing them passwords or long-lived API keys is how accounts get stolen. The safe way is OAuth and OpenID Connect: the other app sends the person to you, you ask them what to allow, and the app gets short-lived tokens that can do only that.

`leptoskit::idp` makes your app that provider. It builds on [sign-in](23-sign-in.md): the same accounts, the same sessions, the same database. You declare what other apps may do as typed scopes, mark the server functions each scope opens, and design the consent screen yourself. A partner's token then calls those functions as the person who said yes, so your [access policies](22-database.md) apply to it unchanged.

The provider runs on your server. The apps that sign in with yours can be another website, a single-page app, a native app on iOS, Android, macOS, Windows, or Linux, a command-line tool or a TV with no browser at all, or another server acting on its own. Your sign-in, consent, and device pages are pages of your own app, built from the same hooks on the web and in your native apps.

## Add the tables

Turn on the `idp` feature next to `auth` and your database, and add the `idp` pack after the `auth` one:

```toml
leptoskit = { version = "…", features = ["idp", "db-postgres", "server"] }
```

```rust
leptoskit::db::packs!(leptoskit_auth, leptoskit_idp);
```

Your next `leptoskit db migration create` adds the tables, in the `idp` namespace. Everything that belongs to a person goes when their account does:

| Table             | What it holds                                                                              |
| ----------------- | ------------------------------------------------------------------------------------------ |
| `clients`         | The apps that may ask: their redirect addresses, their scopes, and a hash of their secret. |
| `grants`          | What each person let each app do.                                                          |
| `authorizations`  | Requests waiting for consent, and authorization codes, as hashes. Codes last a minute.     |
| `families`        | One per sign-in of an app: the refresh tokens that descend from one code.                  |
| `refresh_tokens`  | Hashes of refresh tokens, and when each was used.                                          |
| `signing_keys`    | The public halves of the keys that sign tokens.                                            |
| `pushed_requests` | Requests apps sent ahead from their servers. Each works once, within 10 minutes.           |
| `device_codes`    | Sign-ins started on a CLI or a TV, waiting for someone to enter the code, as hashes.       |
| `assertions`      | The signed assertions apps already used, so none works twice.                              |

## Say what other apps may do

Scopes are the permissions an app asks for. Declare yours once, with a sentence each; the sentence is what your consent screen shows. Put the declaration in code your server functions can see, since they name the scopes:

```rust
leptoskit::rpc::scopes! {
	/// What other apps may do for the people who connect them.
	pub enum Scope {
		/// Read your notes.
		NotesRead = "notes:read",
		/// Create, edit, and delete your notes.
		NotesWrite = "notes:write",
	}
}
```

Then name the scope a server function needs with `#[api(scope = …)]`:

```rust
#[api(scope = Scope::NotesRead)]
#[server]
pub async fn list_my_notes() -> Result<Vec<PublicNote>, ServerFnError> {
	let me = auth::require_signed_in()?;
	let db = auth::request_db()?; // policies see the person who consented
	// …
}
```

A function with no scope stays yours alone: another app's token is refused there, and so is every other route in your app, such as `/auth/*`. A function can name several scopes, and then needs all of them. Inside it, the request is the consenting person's, exactly as if they'd called it themselves; `auth::current_session()?.delegation` says which app is calling, if you need to know.

The scopes also appear in your [published RPC](13-sharing-the-rpc.md): each function lists the scopes it needs under an `oauth2` security scheme, so generated clients know which to ask for.

Every provider also offers the three OpenID Connect scopes: `openid` (sign them in), `profile` (their name and picture, as you release them below), and `email` (their verified email address).

## Start the provider

Turn it on in `leptoskit.toml`. An empty table takes the defaults:

```toml
[auth.idp]
consent_page = "/connect" # where people say yes or no; the default is /consent
device_page = "/device" # where people enter a CLI's or a TV's code
```

| Setting            | Default    | What it's for                                                                                      |
| ------------------ | ---------- | -------------------------------------------------------------------------------------------------- |
| `issuer`           | `origin`   | The address apps discover you at. Leave it to `[auth] origin` unless your sign-in lives elsewhere. |
| `audience`         | the issuer | What access tokens name as their audience: your API's origin, if it isn't the issuer.              |
| `sign_in_page`     | `/sign-in` | Where signed-out people sign in first.                                                             |
| `consent_page`     | `/consent` | Where people answer an app.                                                                        |
| `signed_out_page`  | `/`        | Where people land after an app signs them out, if it named nowhere.                                |
| `device_page`      | `/device`  | Where people enter a device's code.                                                                |
| `rsa_key_env`      | none       | The environment variable holding an RSA key, for apps that need RS256 ID tokens.                   |
| `access_minutes`   | 10         | How long an access token lasts, at most 60.                                                        |
| `id_token_minutes` | 10         | How long an ID token lasts.                                                                        |
| `refresh_days`     | 90         | How long an app stays signed in, however often it refreshes.                                       |
| `signing_key_days` | 30         | How long a signing key signs before the next takes over.                                           |
| `developer_portal` | `false`    | Lets your users register apps of their own.                                                        |
| `apps_per_person`  | 10         | How many apps one person may register in the portal.                                               |

Build the provider next to `Auth`, add its routes inside the session layer, and its own layer outside it:

```rust
let auth = auth::Auth::from_config(&db, &config)?.on_sign_up::<User>().build()?;
let idp = idp::Idp::from_config(&db, &auth, &config)?
	.scopes::<Scope>()
	.claims::<User>()
	.build()?;

// For `leptoskit auth idp client`: carries out the command and exits.
idp.run_command_if_requested().await;

let app = my_app_backend::router()
	.merge(auth.router())
	.merge(idp.router())
	.leptos_routes(&options, routes, move || shell(app_options.clone()))
	.layer(auth.layer())
	.layer(idp.layer());
```

The issuer, the address tokens name and apps discover you at, is `[auth] origin`, such as `https://notes.example`. It must be an `https` origin with no path; `http://localhost` works in development.

`claims::<User>()` releases what apps may learn about a person. The provider fills in `sub`, and with the `email` scope, their verified primary email; your type adds the rest, for the scopes that allow it:

```rust
impl idp::Claims for User {
	async fn claims(db: &Db, account: Id<auth::Account>, scopes: &idp::GrantedScopes, out: &mut idp::ClaimSet) -> idp::Result<()> {
		if scopes.has("profile") {
			if let Some(user) = User::find(db, account).await? {
				out.name(&user.name).preferred_username(&user.handle);
			}
		}
		Ok(())
	}
}
```

`db` acts as the person, so their own policies apply here too.

## Register an app

Before an app can ask, register it. The command builds your server and runs it, so `run_command_if_requested` above must be in `main`:

```console
$ leptoskit auth idp client add "Partner" --redirect https://partner.example/callback --scopes openid,profile,notes:read
Registered Partner.

  client id: 3Jk2…
  auth: client_secret_basic
  redirects: https://partner.example/callback
  scopes: openid profile notes:read

  client secret: q8Zt…

The secret is shown this once; only its hash is kept. Store it in the app's secrets now.
```

- `--public` registers an app that can't keep a secret, such as a native or single-page app, or a command-line tool. It has no secret, and PKCE protects its codes.
- `--redirect` takes the exact addresses the app may be sent back to, compared character for character. They must be `https`, `http` to `127.0.0.1` or `localhost` (any port, for native apps that pick one when they start), or an app's own reverse-domain scheme such as `com.partner.notes:/callback`.
- `--grant` says how the app gets tokens, once per way: `code` (the default) sends people to sign in in a browser; `device` shows a code to enter on another device, for CLIs and TVs; `machine` lets a server act as itself. An app with only `device` or `machine` needs no `--redirect`.
- `--jwks keys.json` registers an app that proves who it is with its own key instead of a secret (`private_key_jwt`). The file holds its public keys as a JWK set, ES256 or RS256, so there's no secret to leak.
- `--rs256` signs the app's ID tokens with RS256, for libraries that check no other kind. The server needs an RSA key for it.
- `--post-logout-redirect` adds where the app may send people after signing them out.
- `--first-party` marks one of your own apps. It sees the account ID as `sub` and skips the consent screen. Every other app sees a different, stable `sub` for the same person, so two apps can't match up their users through you.
- `--introspect` lets a resource server introspect tokens issued to other apps.

`leptoskit auth idp client list` shows every app you registered, and `leptoskit auth idp client remove <client id>` removes one, with every grant and token it holds; everyone who had connected it finds that in their activity. `leptoskit auth idp client rotate-secret <client id>` replaces an app's secret and prints the new one once: the old secret stops working, and so do the tokens the app got as itself.

## Build the consent page

When someone signs in through another app, `/oauth/authorize` checks the request and then sends the browser to one of your pages:

- signed out, to your sign-in page, with `?return_to=/oauth/authorize?…`. Sign them in, then send them to `return_to`;
- signed in, and the app is asking for something they haven't allowed yet, to your consent page with `?request=…`;
- already allowed, straight back to the app with a code.

The fastest start is to let the CLI write the screens, then restyle them. `--idp` adds four screens to the [sign-in screens](23-sign-in.md#start-from-a-template):

```sh
leptoskit generate auth --idp
```

- **Consent**, at `/consent`: the app's name, what it asks for in your scopes' own sentences, and Allow and Cancel. Either answer goes back to the app.
- **Connect a device**, at `/device`: the code field, then the app the code belongs to and what it asks for.
- **Connected apps**, at `/connected-apps` and as a tab in the account bar: each app, what it may do, when it was last used, and Disconnect.
- **Your apps**, at `/developer-apps`: the developer portal's list, a form to register an app, and buttons that make a new secret or delete one.

With `--idp`, `/sign-in` also takes `?return_to=`, so someone sent to sign in by `/oauth/authorize` goes back to it once they're in. Only a path on your own site is followed. Someone who's already signed in is asked whether to go on or to sign in as someone else, so an app that asks people to sign in again can't send them round in a loop.

It also adds `[auth.idp]` to `leptoskit.toml`, the `idp` feature, and, when your app has a server, a `sign_in::provider` function that builds the provider from the config for you to add your scopes and claims to. Running it with `--idp` on an app that already has the sign-in screens brings the files you haven't changed up to date; running it without `--idp` later never takes the screens away.

The screens draw the same on the web and in your native apps, but the consent and sign-in screens sometimes leave your app: back to the app that asked, or on to `/oauth/authorize`. Say how once, after `auth::provide()`. In the browser it's a full page load:

```rust
auth::screens().navigate_with(|address| {
	if let Some(window) = web_sys::window() {
		let _ = window.location().assign(address);
	}
});
```

A native app doesn't need it: apps signing in with yours open your site in a browser, so these screens only ever run there.

Each page is built on hooks from `leptoskit::idp::client`, which you can use directly instead. The consent page reads the request and posts the answer:

```rust
use leptoskit::idp::ConsentDecision;
use leptoskit::idp::client::decide_consent;
use leptoskit::idp::client::use_consent;
use leptoskit::query::use_mutation;

let key = request.clone();
let consent = use_consent(move || key.clone()); // the app, what it asks for, where it goes back to
let decide = use_mutation(
	decide_consent().with_on_success(move |args| navigate(&args.data.redirect_to)),
);

decide.mutate(ConsentDecision { request, approve: true });
```

The hooks behave the same on every platform. In the browser they send the session cookie, which scripts can't read; in a native app they send the access token the [sign-in client](23-sign-in.md) keeps, renewing it when it runs out; while the server renders a page, they read the request's own session. A request expires after 10 minutes, belongs to the person it was made for, and can be answered once. Saying no sends the app `access_denied`.

## What the other app does

The other app sees a standard OpenID Connect provider, so any OAuth 2.1 or OIDC client library works. It discovers everything from `https://notes.example/.well-known/openid-configuration`:

| Route                                   | Does                                                                                           |
| --------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `GET /.well-known/openid-configuration` | Discovery.                                                                                     |
| `GET /oauth/jwks`                       | The public keys that sign tokens.                                                              |
| `GET /oauth/authorize`                  | Starts a sign-in: `response_type=code`, PKCE with `S256`, and the scopes it wants.             |
| `POST /oauth/token`                     | Trades the code, or a refresh token, for tokens.                                               |
| `GET /oauth/userinfo`                   | Claims about the person, with the access token.                                                |
| `POST /oauth/revoke`                    | Revokes a token (RFC 7009).                                                                    |
| `POST /oauth/introspect`                | Says whether a token is live, and what it may do (RFC 7662).                                   |
| `GET /oauth/end_session`                | Signs the person out of the app, and of your site, with the ID token it got (`id_token_hint`). |
| `POST /oauth/par`                       | Pushes a sign-in request from the app's server ahead of the browser (RFC 9126).                |
| `POST /oauth/device_authorization`      | Starts a sign-in for a CLI or a TV (RFC 8628).                                                 |

The token endpoints answer requests from any site, since they read no cookies, so single-page apps can call them directly. `/oauth/authorize` and `/oauth/end_session` also take a form `POST`, as OpenID Connect allows: the browser is sent on to the same page with the same parameters.

Apps authenticate to the token endpoints with their secret (HTTP Basic, or in the form), with a short-lived JWT signed by their own key (`private_key_jwt`: aimed at your issuer or the endpoint, and used once), or not at all if they're public, where PKCE protects the code.

**Pushing the request first.** A sign-in link carries everything the app asks for in the address bar, where browser extensions and logs see it, and where it can be changed on the way. An app with a server can instead post the same parameters to `/oauth/par`, authenticated, and get a `request_uri` that works once. It then sends the browser to `/oauth/authorize?client_id=…&request_uri=…`. Anything `/oauth/authorize` would refuse is refused at the push, while the app's server can still read why.

**Asking for claims in the ID token.** By default an ID token says only who signed in; the rest comes from `userinfo`. An app that wants, say, the email address in the ID token itself passes the `claims` parameter, such as `{"id_token":{"email":null}}`. Only claims the granted scopes release appear, so asking can't reveal more than `userinfo` would.

The answer to a code carries three tokens:

- **An access token**, a JWT that works for 10 minutes. The app sends it to your API as `Authorization: Bearer …`. Your server checks it on every call, including whether the person has since disconnected the app, and resource servers elsewhere can check its signature against the JWKS.
- **A refresh token**, which trades for a new set before the access token runs out. Each one works once. If one is used twice, someone has a copy, so every token from that sign-in stops working and the app must ask the person again.
- **An ID token**, with `openid`: a signed statement of who signed in, for the app itself.

Tokens are signed with ES256. The signing keys rotate every 30 days, and an old key stays in the JWKS until every token it signed has expired. Only the keys' public halves are in your database: each private key is derived from your `AUTH_ENCRYPTION_KEYS` when it's needed, so a copy of the database can't sign anything.

Some OpenID Connect libraries check only RS256 signatures, and OpenID certification asks for RS256 too. For those apps, give the server an RSA key and register them with `--rs256`:

```console
$ openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:3072 -out idp-rsa.pem
```

```toml
[auth.idp]
rsa_key_env = "IDP_RSA_KEY" # holds the PEM file's contents
```

Their ID tokens are then RS256; access tokens stay ES256. The RSA key lives in the environment with your other secrets, never in the database. To rotate it, give the server a new one: the old key stays published until every token it signed has expired.

## Sign in from a CLI or a TV

A command-line tool or a TV app can't show your sign-in page, and asking people to paste a token into a terminal teaches them to paste tokens. The device flow fixes that: the device shows a short code, the person enters it on their phone or computer, where they're already signed in, approves, and the device gets tokens.

Register the device app with `--grant device`, and `--public`, since a CLI on someone's laptop can't keep a secret. Then build the page people enter the code on, at `device_page`. `leptoskit generate auth --idp` writes one; the hooks are `use_device_request` and `decide_device`:

```rust
use leptoskit::idp::DeviceDecision;
use leptoskit::idp::client::decide_device;
use leptoskit::idp::client::use_device_request;

let key = code.clone();
let request = use_device_request(move || key.clone()); // the app on the device, and what it wants
let decide = use_mutation(decide_device());
decide.mutate(DeviceDecision { user_code: code, approve: true });
```

The code reads in any case, with or without its dash. Guessing someone else's is limited to 20 tries per person every 15 minutes, out of 20⁸ codes. Approving records the connection like consent does, so the CLI appears among the person's connected apps.

**Your own app's CLI.** If you [publish your RPC](13-sharing-the-rpc.md) and generate a command line with `leptoskit rpc generate --target cli`, and any of your functions needs a scope, the generated CLI gets `login` and `logout` commands built on this flow:

```console
$ leptoskit auth idp client add "notes-cli" --public --grant device --scopes notes:read,notes:write
$ export NOTES_CLI_CLIENT_ID=3Jk2…
$ notes-cli login
To sign in, open https://notes.example/device and enter the code WDJB-MJHT
Signed in to https://notes.example.
$ notes-cli list-my-notes
```

The CLI keeps its tokens in your config directory (`~/.config/notes-cli/credentials.json`, or under `%APPDATA%` on Windows), readable only by you, renews the access token as it runs out, and sends it with every command. Two commands renewing at once take turns, since a refresh token works once. `logout` revokes the tokens and deletes the file.

## Let a server act as itself

A nightly export job, or a partner's backend, has no person to sign in: it acts as itself. Register it with `--grant machine`. It authenticates with its secret or its key, and asks `/oauth/token` with `grant_type=client_credentials`:

- It gets an access token with your API's own scopes, never `openid`, `profile`, or `email`, and no refresh token: it asks again when the token runs out.
- Its token reaches `#[api(scope = …)]` functions with nobody signed in. `auth::require_signed_in()` refuses it, policies see no one, and `auth::current_session()?.delegation` names the app. Write the functions machines may call with that in mind.
- Revoking one of its tokens, or rotating its secret, ends the token at once on your server.

## Let people see and disconnect apps

People should be able to see which apps they let in, and take that back. `leptoskit generate auth --idp` writes a "Connected apps" page; the hooks are `use_connected_apps` and `disconnect_app`, over these routes:

| Route                       | Body              | Does                                                                     |
| --------------------------- | ----------------- | ------------------------------------------------------------------------ |
| `GET /oauth/grants`         |                   | The person's connected apps, with what each may do and when it was used. |
| `POST /oauth/grants/revoke` | `{ "client_id" }` | Disconnects one. Its tokens stop working straight away.                  |

Each of these lands in the person's activity, the `auth.events` table, naming the app: connecting an app or letting it do more (`app_connected`); disconnecting it, or the app being removed (`app_disconnected`); and a code or refresh token being used twice, which ends that sign-in (`app_token_reused`). Show them on a security page, so people notice a sign-in they didn't make: `auth::Event::query()` on `auth::request_db()?` returns only the signed-in person's own.

## Let developers register their own apps

If other people build on your API, they need client IDs without asking you for each one. Turn on the developer portal:

```toml
[auth.idp]
developer_portal = true
apps_per_person = 10
```

Signed-in people can then register, change, and delete apps of their own. `leptoskit generate auth --idp` writes a page for it; the hooks are `use_my_apps`, `register_app`, `update_app`, `rotate_app_secret`, and `delete_app`, over `/oauth/apps`.

- Each person sees only their own apps, and `leptoskit auth idp client list` shows only yours.
- A portal app is never first-party, never skips consent, and can't introspect other apps' tokens; those stay for apps you register yourself.
- A secret is shown once, when it's made.
- Taking a scope away from an app takes it from everyone who connected it, and ends the sign-ins that carried it. Deleting an app disconnects it from everyone, and their activity says so.

## What isn't there yet

- DPoP-bound tokens and dynamic client registration.
- PKCE is required for every app, including confidential ones. Some older libraries, and the OpenID Foundation's basic certification tests, sign in without it; those can't use your provider yet.
- The generated CLI keeps its tokens in a file in your config directory, not in the operating system's keychain.
