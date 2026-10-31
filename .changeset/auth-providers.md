---
"release": minor
---

# Sign in with Google, Apple, Microsoft, and GitHub

`leptoskit::auth` signs people in with other sites (the `auth-providers` feature on your server): Google, Apple, Microsoft, GitHub, and any OpenID Connect provider, such as Okta, Auth0, or Keycloak.

- **Add one** with `leptoskit auth provider add google`, which writes `[auth.providers.google]` and prints its setup steps. Secrets stay in environment variables.
- **Sign in** with `sign_in_with_provider()`; the page comes back with the outcome, which `provider_outcome()` reads. Every flow uses PKCE, `state`, and a nonce held on the server, bound to the browser by an `HttpOnly` cookie, and ID tokens are checked against the provider's published keys.
- **Accounts are never joined by email.** When a provider's address belongs to another account, the person chooses: sign in to it, or `continue_provider_sign_up()` for a separate one.
- **Connect and disconnect** providers with `link_provider()`, `use_identities()`, and `unlink_identity()`. When a provider's account already has its own account here, `use_merge_offer()` describes it and `merge_accounts()` moves its sign-in methods into this one, after the person confirms and proves both accounts.
- The functions exist on every platform; native apps answer `unsupported` until the system-browser plugin arrives.
