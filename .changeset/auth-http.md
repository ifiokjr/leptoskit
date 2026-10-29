---
"release": minor
---

# Sign-in over HTTP

With the `server` feature, `leptoskit::auth` serves sign-in.

- **`auth.router()`** adds JSON routes under `/auth`. They send and check email codes, report the current session, sign out, and list and revoke a person's sessions.
- **`auth.layer()`** resolves every request's session, from an `HttpOnly` `__Host-session` cookie on the web or a bearer token from a native app. It hands handlers a database that acts for the signed-in person.
- **Other sites** can't make state-changing requests: the layer refuses them by checking `Origin` and `Sec-Fetch-Site`.
- **Handlers** take `auth::SignedIn` (`401` when signed out). Server functions call `auth::require_signed_in()` and `auth::request_db()`.
- **Development:** `.dev_mailbox()` shows outgoing email at `/_leptoskit/mail`, and `.dev_sign_in()` signs in to any address without a code. Neither exists in release builds.
