# X

Lets people sign in with their X (formerly Twitter) account. X uses OAuth 2 with PKCE, and leptoskit reads who they are from X's API. Last checked against X's documentation on 31 October 2026.

## Set it up

1. Run `leptoskit auth provider add x`. It adds:

   ```toml
   [auth.providers.x]
   client_id_env = "X_CLIENT_ID"
   client_secret_env = "X_CLIENT_SECRET"
   ```

2. In the [X developer portal](https://developer.x.com/en/portal/dashboard), open your app's User authentication settings, and choose OAuth 2.0 with the type "Web App, Automated App or Bot".
3. Add the Callback URIs `https://myapp.com/auth/callback/x`, and `http://localhost:3000/auth/callback/x` for development.
4. Set `X_CLIENT_ID` and `X_CLIENT_SECRET` to the OAuth 2.0 Client ID and Client Secret, not the API key and secret.
5. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

The person's numeric X ID, which never changes, their name, their username, and their picture. leptoskit asks for the `tweet.read` and `users.read` scopes, since X needs both to read a profile, and authenticates to X's token endpoint with HTTP Basic, as X requires for a confidential client.

X never shares an email address that it has checked, so accounts made with X start without one, and an X sign-in never matches another account by address.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
