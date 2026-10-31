---
"release": minor
---

# Bind tokens to apps' keys, and let apps register

`leptoskit::idp` gains two opt-in features, both off by default.

- **DPoP (RFC 9449):** with `dpop = true` in `[auth.idp]`, an app that sends a `DPoP` proof gets tokens bound to its key: the access token works at your API and `userinfo` only as `Authorization: DPoP` with a fresh proof from that key, and a public app's refresh token only with one too. A stolen token is useless without the key. `leptoskit auth idp client add --dpop` requires it of an app. Introspection reports the key as `cnf`, and discovery lists the proof algorithms.
- **Dynamic client registration (RFC 7591 and 7592):** with `dynamic_registration = true`, apps register themselves at `POST /oauth/register`, then read, change, and delete their registration with the token it returns. `registration_token_env` names an initial access token apps must send; without one, registration is open and limited to 10 apps an hour from one address. Self-registered apps are never first-party, can't introspect other apps' tokens, and are marked in `client list`.
