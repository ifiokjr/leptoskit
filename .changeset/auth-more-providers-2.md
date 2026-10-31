---
"release": minor
---

# Sign in with Twitch, Spotify, Reddit, Amazon, and Dropbox

Five more providers have built-in definitions, each with a setup page and `leptoskit auth provider add <id>`:

- **Twitch**, through its OpenID Connect sign-in, asking for the address and username by name with Twitch's `claims` parameter.
- **Spotify**, whose addresses are never trusted, since Spotify doesn't check them.
- **Reddit**, which shares no address. Its requests name your app in `User-Agent`, as Reddit asks, from `[auth] app_name`: the app's name only, never anything about the person.
- **Amazon**, through Login with Amazon; it doesn't say whether it checked an address, so none is trusted.
- **Dropbox**, through its OpenID Connect sign-in, with `email_verified`.

An OpenID Connect provider whose ID tokens have no `name` now gets one from `given_name` and `family_name`.

If you configured one of these IDs as a generic OpenID Connect provider with an `issuer`, add `kind = "oidc"` to keep it that way.
