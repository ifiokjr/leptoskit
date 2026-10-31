# Notion

Lets people sign in with their Notion account, through a public integration. Last checked against Notion's documentation on 31 October 2026.

## Set it up

1. Run `leptoskit auth provider add notion`. It adds:

   ```toml
   [auth.providers.notion]
   client_id_env = "NOTION_CLIENT_ID"
   client_secret_env = "NOTION_CLIENT_SECRET"
   ```

2. In [Notion's integrations page](https://www.notion.so/profile/integrations), create a public integration.
3. Add the Redirect URIs `https://myapp.com/auth/callback/notion`, and `http://localhost:3000/auth/callback/notion` for development.
4. Set `NOTION_CLIENT_ID` (the OAuth client ID) and `NOTION_CLIENT_SECRET`.
5. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

Notion grants access to one workspace at a time, and the grant names the person who connected it. leptoskit signs in that person: their Notion user ID, which is the same in every workspace, their name, their picture, and their email address. A grant owned by anything but a person, such as a whole workspace, signs no one in.

Notion doesn't say whether it checked the address, so leptoskit keeps it on the connected account for display, but never stores it as the account's own or uses it to find another account. Accounts made with Notion start without an email address.

Notion's sign-in has no scopes, and doesn't take PKCE. The code is still bound to the browser that started the sign-in by its `state` and an `HttpOnly` cookie, and only your server can trade it, with the client secret.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
