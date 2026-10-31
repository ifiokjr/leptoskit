# Reddit

Lets people sign in with their Reddit account. Reddit uses OAuth 2, and leptoskit reads who they are from Reddit's API. Last checked against Reddit's documentation on 31 October 2026.

## Set it up

1. Run `leptoskit auth provider add reddit`. It adds:

   ```toml
   [auth.providers.reddit]
   client_id_env = "REDDIT_CLIENT_ID"
   client_secret_env = "REDDIT_CLIENT_SECRET"
   ```

2. In [Reddit's app preferences](https://www.reddit.com/prefs/apps), create an app of type "web app" with the redirect uri `https://myapp.com/auth/callback/reddit`. Reddit takes one redirect URI per app, so create a second app for development, with `http://localhost:3000/auth/callback/reddit`.
3. Set `REDDIT_CLIENT_ID` (shown under the app's name) and `REDDIT_CLIENT_SECRET`.
4. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

The person's Reddit account ID, which never changes, their username, and their picture. leptoskit asks for the `identity` scope only, for a grant that lasts as long as the sign-in.

Reddit asks every app to say who it is in its requests' `User-Agent` header, and slows down apps that don't. leptoskit sends `web:<your app>:leptoskit-auth/<version>`, built from `[auth] app_name` (or `[app] name`): your app's name only, never anything about the person signing in.

Reddit shares no email address, so accounts made with Reddit start without one.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
