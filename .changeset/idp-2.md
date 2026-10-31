---
"release": minor
---

# Sign in from CLIs, TVs, servers, and your users' apps

`leptoskit::idp` now covers the ways in beyond a browser's code flow, and the pieces apps need around it.

- **The device flow** (RFC 8628) for command-line tools and TVs: the device shows a code, the person enters it on your device page and approves. CLIs generated with `leptoskit rpc generate --target cli` get `login` and `logout` built on it when any function needs a scope, and keep their tokens in a file only you can read.
- **Apps acting as themselves** with `client_credentials`: access tokens with your API's own scopes and no person, which reach `#[api(scope = …)]` functions signed out.
- **Pushed authorization requests** (RFC 9126) at `/oauth/par`, form `POST`s to `/oauth/authorize` and `/oauth/end_session`, and the `claims` parameter for ID-token claims.
- **`private_key_jwt`** client authentication with ES256 or RS256 keys, and **RS256 ID tokens** for apps registered for them, from an RSA key in the environment.
- **A developer portal**, off by default: signed-in people register, change, rotate, and delete apps of their own through `/oauth/apps`.
- **Headless hooks** in `leptoskit::idp::client`, the same on the web, in native apps, and while the server renders: consent, device codes, connected apps, and the portal. `leptoskit generate auth --idp` adds consent, connect-a-device, connected-apps, and developer-portal screens and routes to the sign-in screens, a `?return_to=` on `/sign-in` so people go back to the app that sent them, `[auth.idp]`, and the provider's builder on the server.
- **The audit trail** records connecting and disconnecting apps, reused codes and refresh tokens, and portal changes in `auth.events`, naming the app. Other crates record their own events with `Auth::record`.
- **`[auth.idp]` in `leptoskit.toml`** configures pages, lifetimes, the RSA key, and the portal; `Idp::from_config` reads it.
- **`leptoskit auth idp client add`** takes `--grant code|device|machine`, `--jwks`, and `--rs256`, and `client rotate-secret` replaces a secret.
