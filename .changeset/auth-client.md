---
"release": minor
---

# A headless sign-in client

`leptoskit::auth::client` supplies the behaviour of a sign-in screen; the design stays yours.

- **Query and mutation scopes** report the session, email a code, sign in with it, and sign out: `use_auth()` for the session, plus `send_code()`, `verify_code()` and `sign_out_mutation()`. Signing in or out updates every screen that reads `use_auth()`.
- **Errors** arrive as `ErrorResponse`, with the same codes as the `/auth` routes, such as `invalid_code` and `rate_limited`.
