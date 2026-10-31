# Twitch

Lets people sign in with their Twitch account, through Twitch's OpenID Connect sign-in. Last checked against Twitch's documentation on 31 October 2026.

## Set it up

1. Run `leptoskit auth provider add twitch`. It adds:

   ```toml
   [auth.providers.twitch]
   client_id_env = "TWITCH_CLIENT_ID"
   client_secret_env = "TWITCH_CLIENT_SECRET"
   ```

2. In the [Twitch developer console](https://dev.twitch.tv/console/apps), register an application with the category "Website Integration" and the client type "Confidential".
3. Add the OAuth Redirect URLs `https://myapp.com/auth/callback/twitch`, and `http://localhost:3000/auth/callback/twitch` for development.
4. Set `TWITCH_CLIENT_ID` and `TWITCH_CLIENT_SECRET`.
5. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

Twitch's ID token: the person's Twitch user ID, their username, picture, and email address with Twitch's `email_verified` flag. Twitch only puts the address and username in the token when the sign-in asks for each by name, so leptoskit sends Twitch's `claims` parameter for them, along with the `openid` and `user:read:email` scopes.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
