# Spotify

Lets people sign in with their Spotify account. Spotify uses OAuth 2 rather than OpenID Connect, so leptoskit reads who they are from Spotify's Web API. Last checked against Spotify's documentation on 31 October 2026.

## Set it up

1. Run `leptoskit auth provider add spotify`. It adds:

   ```toml
   [auth.providers.spotify]
   client_id_env = "SPOTIFY_CLIENT_ID"
   client_secret_env = "SPOTIFY_CLIENT_SECRET"
   ```

2. In the [Spotify developer dashboard](https://developer.spotify.com/dashboard), create an app and tick Web API.
3. Add the Redirect URIs `https://myapp.com/auth/callback/spotify`, and `http://localhost:3000/auth/callback/spotify` for development. Spotify accepts plain `http` only for loopback addresses, so use `http://127.0.0.1:3000/auth/callback/spotify` if it refuses `localhost`.
4. Set `SPOTIFY_CLIENT_ID` and `SPOTIFY_CLIENT_SECRET`.
5. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

The person's Spotify user ID, which never changes, their display name, their picture, and their email address. leptoskit asks for the `user-read-email` scope.

Spotify doesn't check that people own the address on their account, so leptoskit keeps it on the connected account for display, but never stores it as the account's own or uses it to find another account. Accounts made with Spotify start without an email address.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
