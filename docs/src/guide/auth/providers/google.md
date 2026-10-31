# Google

Lets people sign in with their Google account, including Google Workspace accounts. Last checked against Google's documentation on 30 October 2026.

## Set it up

1. Run `leptoskit auth provider add google`. It adds this to `leptoskit.toml` and prints the steps below with your addresses filled in:

   ```toml
   [auth.providers.google]
   client_id_env = "GOOGLE_CLIENT_ID"
   client_secret_env = "GOOGLE_CLIENT_SECRET"
   ```

2. In the [Google Cloud console](https://console.cloud.google.com/apis/credentials), create an OAuth client ID of type "Web application".
3. Add your redirect URIs: `https://myapp.com/auth/callback/google` (your `[auth] origin` followed by `/auth/callback/google`), and for development `http://localhost:3000/auth/callback/google`.
4. Set `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` in your server's environment.
5. Turn on the `auth-providers` feature on your server's `leptoskit` dependency, and run `leptoskit auth doctor`.

## What sign-in gets

Google's OpenID Connect ID token: a stable ID for the person, their name and picture, and their email address with Google's `email_verified` flag. A verified address is stored as the account's; it never signs in to another account that has the same address. leptoskit asks for the `openid`, `email`, and `profile` scopes; set `scopes` in the table to change them.

## Platforms

Signing in with Google works on the web. Native apps answer `unsupported` until the system-browser plugin arrives; see [Sign-in](../../23-sign-in.md#sign-in-with-other-sites).
