# Yahoo

Lets people sign in with their Yahoo account, through Yahoo's OpenID Connect sign-in. Last checked against Yahoo's documentation on 31 October 2026.

## Set it up

1. Run `leptoskit auth provider add yahoo`. It adds:

   ```toml
   [auth.providers.yahoo]
   client_id_env = "YAHOO_CLIENT_ID"
   client_secret_env = "YAHOO_CLIENT_SECRET"
   ```

2. In [Yahoo's developer site](https://developer.yahoo.com/apps/create), create an app with OpenID Connect permissions for Email and Profile.
3. Set its Redirect URI(s) to `https://myapp.com/auth/callback/yahoo`, and `http://localhost:3000/auth/callback/yahoo` for development.
4. Set `YAHOO_CLIENT_ID` and `YAHOO_CLIENT_SECRET`.
5. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

Yahoo's ID token: a stable ID for the person, their name and picture, and their email address with Yahoo's `email_verified` flag. leptoskit asks for the `openid`, `email`, and `profile` scopes.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
