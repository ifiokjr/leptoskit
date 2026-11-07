---
"release": minor
---

# Services know who is signed in

A service can now tell who is calling, so it can keep per-person data such as "my events" and per-person live queries. The app vouches for the person: its server mints a token that names them (their `Actor`, such as a `leptoskit_auth` account), lasts five minutes, and works only at the one service it's for. The key comes from the token `leptoskit` already gives the app and each service, through HKDF with its own label and the service's name, so nothing new has to be deployed.

- **Configuration.** `forward_identity` in a `[services.<name>]` table turns it on or off. It defaults to `true` for first-party services and `false` for isolated ones, and only `leptoskit.toml` can set it, never a service's manifest. `leptoskit dev`, `leptoskit serve`, Compose, Kubernetes, `leptoskit infra`, and the Docker steps give the app `LEPTOSKIT_SERVICE_<NAME>_IDENTITY=forward` for each such service, and `leptoskit services` lists it.
- **Browsers and native apps.** Before the first call to a service, the client asks the app for that service's token with the built-in action `rpc.service_token`, signed the way the app already is. It sends the token with that service's HTTP calls and on its live connection only, renews it before it runs out or when the service refuses it, and forgets it on `rpc::client::reauthenticate()`, which `leptoskit_auth`'s client calls on every sign-in and sign-out. With no token to give, calls go anonymously.
- **Server rendering.** A page rendered for a signed-in person calls the service as that person, so the page holds what their browser would fetch. `call_as_app` is still the only way to call as the app.
- **In the service.** `ServiceCallers::for_service` accepts these tokens and identifies the person as the caller `person:<id>`, with their `Actor` and a `ForwardedIdentity` in the context. A token for another service, a changed one, or an expired one is refused with `401`. A live connection keeps the person until their token runs out and asks the client for a new one a minute before.

`rpc::server::service_identity_token` and `IdentityKey` mint and check tokens by hand. `examples/analytics` gains a live `mine` query of the signed-in person's own events, and records a signed-in person's events under them.
