<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_idp

> Sign in with your app: an OAuth 2.1 and OpenID Connect provider for leptoskit apps, built on their own database, sessions, and access policies.

Let other apps, including other leptoskit apps, native apps, and command-line tools, sign people in with yours and call your API for them. The provider keeps its state in your database as the `idp` schema pack, next to `leptoskit_auth`'s:

```rust,ignore
leptoskit::db::packs!(leptoskit_auth, leptoskit_idp);
```

Declare what other apps may do, and which server functions each scope opens:

```rust,ignore
leptoskit::rpc::scopes! {
	pub enum Scope {
		/// Read your notes.
		NotesRead = "notes:read",
	}
}

#[api(scope = Scope::NotesRead)]
#[server]
pub async fn list_my_notes() -> Result<Vec<PublicNote>, ServerFnError> { /* runs as the person who consented */ }
```

Then serve it next to sign-in:

```rust,ignore
let idp = idp::Idp::builder(&db, &auth).scopes::<Scope>().consent_page("/connect").build()?;
idp.run_command_if_requested().await;   // for `leptoskit auth idp client add|list|remove`

let app = router
	.merge(auth.router())
	.merge(idp.router())   // /oauth/authorize, consent, connected apps, sign-out
	.layer(auth.layer())
	.layer(idp.layer());   // discovery, JWKS, token, userinfo, revoke, introspect
```

- **The code flow only,** with PKCE `S256` required and redirect addresses compared exactly.
- **Access tokens** are 10-minute ES256 JWTs (RFC 9068). **Refresh tokens** are opaque, stored hashed, and work once; a second use ends every token from that sign-in.
- **Signing keys** rotate, and only their public halves are stored: the private keys derive from `AUTH_ENCRYPTION_KEYS`.
- **Pairwise subjects:** each app sees its own `sub` for a person, unless it's one of your own.
- **Consent is headless:** your page reads `GET /oauth/consent` and posts the answer.
- **Opt-in hardening and openness:** DPoP (RFC 9449) binds an app's tokens to its key, and dynamic registration (RFC 7591 and 7592) lets apps register themselves, with or without an initial access token. Both are off by default.

The tables and the wire types build everywhere. `Idp` needs the `server` feature and a database, `postgres` or `sqlite`; its routes need `axum`.
