---
"release": minor
---

# Let other apps sign in with yours

`leptoskit::idp` turns your app into an OAuth 2.1 and OpenID Connect provider, so other websites, native apps, and command-line tools can offer "Sign in with" your app and call your API for the people who allow it.

- **The `idp` schema pack** keeps registered apps, consent grants, codes, refresh-token families, and the public halves of signing keys in your own database, next to the `auth` pack.
- **Standard endpoints:** discovery, JWKS, the authorization code flow with PKCE (`S256` only, exact redirect addresses), the token endpoint, `userinfo`, revocation, introspection, and RP-initiated logout.
- **Tokens:** 10-minute ES256 access tokens (RFC 9068), ID tokens, and refresh tokens that work once; using one twice ends every token from that sign-in. Each app sees its own `sub` for a person.
- **Your consent page:** `/oauth/authorize` sends people to a page you design, which reads the request and posts the answer as JSON. People see and disconnect apps through `/oauth/grants`.
- **Typed scopes:** declare them with `leptoskit::rpc::scopes!`, and require them with `#[api(scope = Scope::NotesRead)]`. A partner's token reaches only those functions, which run as the person who allowed it, so access policies apply unchanged. The OpenAPI document lists each function's scopes.
- **`leptoskit auth idp client add`, `list`, and `remove`** register and remove apps, printing a confidential app's secret once.
