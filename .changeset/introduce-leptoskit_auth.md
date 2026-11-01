---
leptoskit_auth: breaking
---

# leptoskit_auth: sign-in and sessions

`leptoskit_auth` is sign-in for leptoskit apps: the auth schema pack, sessions, email codes, recovery, passkeys, providers, and the signed-in user that access policies read.

It owns the server-side session and the shared client, and uses the passkey and web-auth plugins for native sign-in. `leptoskit_idp` turns the same accounts into an OIDC provider.
