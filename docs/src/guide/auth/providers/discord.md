# Discord

Lets people sign in with their Discord account. Discord uses OAuth 2 rather than OpenID Connect, so leptoskit reads who they are from Discord's API. Last checked against Discord's documentation on 31 October 2026.

## Set it up

1. Run `leptoskit auth provider add discord`. It adds:

   ```toml
   [auth.providers.discord]
   client_id_env = "DISCORD_CLIENT_ID"
   client_secret_env = "DISCORD_CLIENT_SECRET"
   ```

2. In the [Discord developer portal](https://discord.com/developers/applications), create an application and open its OAuth2 page.
3. Add the Redirects `https://myapp.com/auth/callback/discord`, and `http://localhost:3000/auth/callback/discord` for development.
4. Set `DISCORD_CLIENT_ID` and `DISCORD_CLIENT_SECRET`.
5. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

The person's Discord ID, which never changes, their display name and username, and their email address with Discord's `verified` flag. leptoskit asks for the `identify` and `email` scopes. A verified address is stored as the account's; an unverified one is kept on the connected account for display only.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
