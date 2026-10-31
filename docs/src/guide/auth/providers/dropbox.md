# Dropbox

Lets people sign in with their Dropbox account, through Dropbox's OpenID Connect sign-in. Last checked against Dropbox's documentation on 31 October 2026.

## Set it up

1. Run `leptoskit auth provider add dropbox`. It adds:

   ```toml
   [auth.providers.dropbox]
   client_id_env = "DROPBOX_CLIENT_ID"
   client_secret_env = "DROPBOX_CLIENT_SECRET"
   ```

2. In the [Dropbox App Console](https://www.dropbox.com/developers/apps), create an app with scoped access, and on its Permissions tab turn on `openid`, `email`, and `profile`.
3. On its Settings tab, add the Redirect URIs `https://myapp.com/auth/callback/dropbox`, and `http://localhost:3000/auth/callback/dropbox` for development.
4. Set `DROPBOX_CLIENT_ID` (the app key) and `DROPBOX_CLIENT_SECRET` (the app secret).
5. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

Dropbox's ID token: the person's Dropbox account ID, their name (from its given and family names), and their email address with Dropbox's `email_verified` flag.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
