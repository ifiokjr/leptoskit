# Zoom

Lets people sign in with their Zoom account. Zoom uses OAuth 2, and leptoskit reads who they are from Zoom's API. Last checked against Zoom's documentation on 31 October 2026.

## Set it up

1. Run `leptoskit auth provider add zoom`. It adds:

   ```toml
   [auth.providers.zoom]
   client_id_env = "ZOOM_CLIENT_ID"
   client_secret_env = "ZOOM_CLIENT_SECRET"
   ```

2. In the [Zoom App Marketplace](https://marketplace.zoom.us/develop/create), create a General App with user-managed OAuth, and add the scope `user:read:user`.
3. Add the OAuth Redirect URLs `https://myapp.com/auth/callback/zoom`, and `http://localhost:3000/auth/callback/zoom` for development, and the same addresses to the OAuth Allow List.
4. Set `ZOOM_CLIENT_ID` and `ZOOM_CLIENT_SECRET`.
5. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

The person's Zoom user ID, their display name, their picture, and their email address with Zoom's `verified` flag. A verified address is stored as the account's; an unverified one is kept on the connected account for display only.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
