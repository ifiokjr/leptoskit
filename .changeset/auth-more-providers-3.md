---
"release": minor
---

# Sign in with Bitbucket, Notion, Zoom, TikTok, and Yahoo

The last five of the twenty providers leptoskit planned have built-in definitions, each with a setup page and `leptoskit auth provider add <id>`:

- **Bitbucket**, trusting the primary address only when Bitbucket confirmed it.
- **Notion**, signing in the person who connected the integration to their workspace. A grant owned by anything but a person signs no one in.
- **Zoom**, trusting an address when Zoom's `verified` flag says so.
- **TikTok**, through Login Kit for the web, with your app's client key. TikTok shares no address.
- **Yahoo**, through its OpenID Connect sign-in.

Notion and TikTok's web sign-ins don't take PKCE; there the sign-in is bound to the browser by its `state` and an `HttpOnly` cookie, and only your server can trade the code, with the client secret.

If you configured one of these IDs as a generic OpenID Connect provider with an `issuer`, add `kind = "oidc"` to keep it that way.
