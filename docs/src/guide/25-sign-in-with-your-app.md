# Sign in with your app

Once people have accounts in your app, other apps start asking for them: a partner's site wants a "Sign in with Notes" button, your own command-line tool wants to act for its user, or someone builds an integration on your API. Handing them passwords or long-lived API keys is how accounts get stolen. The safe way is OAuth and OpenID Connect: the other app sends the person to you, you ask them what to allow, and the app gets short-lived tokens that can do only that.

`leptoskit::idp` makes your app that provider. It builds on [sign-in](23-sign-in.md): the same accounts, the same sessions, the same database. You declare what other apps may do as typed scopes, mark the server functions each scope opens, and design the consent screen yourself. A partner's token then calls those functions as the person who said yes, so your [access policies](22-database.md) apply to it unchanged.

The provider runs on your server. The apps that sign in with yours can be anything that opens a browser: another website, a single-page app, a native app on iOS, Android, macOS, Windows, or Linux, or a command-line tool. Your sign-in and consent pages are ordinary web pages on your site.

## Add the tables

Turn on the `idp` feature next to `auth` and your database, and add the `idp` pack after the `auth` one:

```toml
leptoskit = { version = "…", features = ["idp", "db-postgres", "server"] }
```

```rust
leptoskit::db::packs!(leptoskit_auth, leptoskit_idp);
```

Your next `leptoskit db migration create` adds the tables, in the `idp` namespace. Everything that belongs to a person goes when their account does:

| Table            | What it holds                                                                              |
| ---------------- | ------------------------------------------------------------------------------------------ |
| `clients`        | The apps that may ask: their redirect addresses, their scopes, and a hash of their secret. |
| `grants`         | What each person let each app do.                                                          |
| `authorizations` | Requests waiting for consent, and authorization codes, as hashes. Codes last a minute.     |
| `families`       | One per sign-in of an app: the refresh tokens that descend from one code.                  |
| `refresh_tokens` | Hashes of refresh tokens, and when each was used.                                          |
| `signing_keys`   | The public halves of the keys that sign tokens.                                            |

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

Build the provider next to `Auth`, add its routes inside the session layer, and its own layer outside it:

```rust
let auth = auth::Auth::from_config(&db, &config)?.on_sign_up::<User>().build()?;
let idp = idp::Idp::builder(&db, &auth)
	.scopes::<Scope>()
	.claims::<User>()
	.sign_in_page("/sign-in")
	.consent_page("/connect")
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

- `--public` registers an app that can't keep a secret, such as a native or single-page app. It has no secret, and PKCE protects its codes.
- `--redirect` takes the exact addresses the app may be sent back to, compared character for character. They must be `https`, `http` to `127.0.0.1` or `localhost` (any port, for native apps that pick one when they start), or an app's own reverse-domain scheme such as `com.partner.notes:/callback`.
- `--post-logout-redirect` adds where the app may send people after signing them out.
- `--first-party` marks one of your own apps. It sees the account ID as `sub` and skips the consent screen. Every other app sees a different, stable `sub` for the same person, so two apps can't match up their users through you.
- `--introspect` lets a resource server introspect tokens issued to other apps.

`leptoskit auth idp client list` shows every app, and `leptoskit auth idp client remove <client id>` removes one, with every grant and token it holds.

## Build the consent page

When someone signs in through another app, `/oauth/authorize` checks the request and then sends the browser to one of your pages:

- signed out, to your sign-in page, with `?return_to=/oauth/authorize?…`. Sign them in, then send them to `return_to`;
- signed in, and the app is asking for something they haven't allowed yet, to your consent page with `?request=…`;
- already allowed, straight back to the app with a code.

The consent page is yours to design. It reads the request, shows it, and posts the answer:

```rust
#[component]
pub fn Connect() -> impl IntoView {
	let request = use_query_map().with_untracked(|query| query.get("request")).unwrap_or_default();
	let details = LocalResource::new({
		let request = request.clone();
		move || fetch_json::<idp::ConsentRequest>(format!("/oauth/consent?request={request}"))
	});
	let answer = move |approve: bool| {
		let request = request.clone();
		spawn_local(async move {
			let decision = idp::ConsentDecision { request, approve };
			let next: idp::ConsentRedirect = post_json("/oauth/consent", &decision).await;
			window().location().set_href(&next.redirect_to).ok();
		});
	};

	view! {
		<Suspense>
			{move || details.get().map(|details| view! {
				<h1>{format!("{} wants to use your account", details.app.name)}</h1>
				<ul>
					{details.scopes.into_iter().map(|scope| view! { <li>{scope.description}</li> }).collect_view()}
				</ul>
				<p>{format!("You'll be sent back to {}.", details.redirect_host)}</p>
				<button on:click=move |_| answer(false)>"Cancel"</button>
				<button on:click=move |_| answer(true)>"Allow"</button>
			})}
		</Suspense>
	}
}
```

`fetch_json` and `post_json` stand for your app's own same-origin `fetch` helpers; both requests carry the session cookie. A request expires after 10 minutes, belongs to the person it was made for, and can be answered once. Saying no sends the app `access_denied`.

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

The token endpoints answer requests from any site, since they read no cookies, so single-page apps can call them directly.

The answer to a code carries three tokens:

- **An access token**, a JWT that works for 10 minutes. The app sends it to your API as `Authorization: Bearer …`. Your server checks it on every call, including whether the person has since disconnected the app, and resource servers elsewhere can check its signature against the JWKS.
- **A refresh token**, which trades for a new set before the access token runs out. Each one works once. If one is used twice, someone has a copy, so every token from that sign-in stops working and the app must ask the person again.
- **An ID token**, with `openid`: a signed statement of who signed in, for the app itself.

Tokens are signed with ES256. The signing keys rotate every 30 days, and an old key stays in the JWKS until every token it signed has expired. Only the keys' public halves are in your database: each private key is derived from your `AUTH_ENCRYPTION_KEYS` when it's needed, so a copy of the database can't sign anything.

## Let people see and disconnect apps

People should be able to see which apps they let in, and take that back. Two routes back a "Connected apps" screen:

| Route                       | Body              | Does                                                                     |
| --------------------------- | ----------------- | ------------------------------------------------------------------------ |
| `GET /oauth/grants`         |                   | The person's connected apps, with what each may do and when it was used. |
| `POST /oauth/grants/revoke` | `{ "client_id" }` | Disconnects one. Its tokens stop working straight away.                  |

## What isn't there yet

- Pushed authorization requests, the device flow for TVs and command-line tools, and the `client_credentials` grant for machine-to-machine calls.
- `private_key_jwt` client authentication; apps use a secret, or PKCE alone if they're public.
- DPoP-bound tokens and dynamic client registration.
- ID tokens signed with RS256. Some older OpenID Connect libraries accept only RS256; check that the other app's library accepts ES256.
- `POST` to `/oauth/authorize` and `/oauth/end_session`; use `GET`.
- A developer portal where your users register their own apps.
