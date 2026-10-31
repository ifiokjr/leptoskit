# Facebook

Lets people sign in with their Facebook account. Facebook uses OAuth 2 rather than OpenID Connect, so leptoskit reads who they are from the Graph API, at a version you pin. Last checked against Meta's documentation on 31 October 2026, with Graph API v26.0.

## Set it up

1. Run `leptoskit auth provider add facebook`. It adds:

   ```toml
   [auth.providers.facebook]
   client_id_env = "FACEBOOK_CLIENT_ID"
   client_secret_env = "FACEBOOK_CLIENT_SECRET"
   api_version = "v26.0"
   ```

2. In [Meta for Developers](https://developers.facebook.com/apps), create an app with the "Authenticate and request data from users with Facebook Login" use case.
3. Under Facebook Login's settings, add the Valid OAuth Redirect URIs `https://myapp.com/auth/callback/facebook`, and `http://localhost:3000/auth/callback/facebook` for development.
4. Set `FACEBOOK_CLIENT_ID` (the app ID) and `FACEBOOK_CLIENT_SECRET` (the app secret).
5. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

`api_version` pins the Graph API version, so a new release never changes sign-in under you. Facebook retires each version about two years after the next one ships; `leptoskit auth doctor` warns when yours is older than the one leptoskit was checked against.

## What sign-in gets

The person's Facebook ID for your app, which never changes, their name, their picture, and their email address when they have one and grant it. leptoskit asks for the `public_profile` and `email` permissions.

Facebook doesn't say whether it checked an address, so leptoskit keeps it on the connected account for display, but never stores it as the account's own or uses it to find another account. Accounts made with Facebook start without an email address.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
