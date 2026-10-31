# TikTok

Lets people sign in with their TikTok account, through Login Kit for the web. Last checked against TikTok's documentation on 31 October 2026.

## Set it up

1. Run `leptoskit auth provider add tiktok`. It adds:

   ```toml
   [auth.providers.tiktok]
   client_id_env = "TIKTOK_CLIENT_ID"
   client_secret_env = "TIKTOK_CLIENT_SECRET"
   ```

2. In [TikTok for Developers](https://developers.tiktok.com/apps), create an app and add Login Kit for the web, with the `user.info.basic` scope.
3. Add the Redirect URI `https://myapp.com/auth/callback/tiktok`. TikTok only accepts `https` addresses, so try it during development through a tunnel to your machine, or on a deployed site.
4. Set `TIKTOK_CLIENT_ID` to the app's client key, and `TIKTOK_CLIENT_SECRET` to its client secret.
5. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

The person's TikTok `open_id`, which is stable for your app, their display name, and their avatar.

TikTok shares no email address, so accounts made with TikTok start without one. Its web sign-in doesn't take PKCE: the code is bound to the browser that started the sign-in by its `state` and an `HttpOnly` cookie, and only your server can trade it, with the client secret.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
