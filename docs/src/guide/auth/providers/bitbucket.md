# Bitbucket

Lets people sign in with their Bitbucket Cloud account. Bitbucket uses OAuth 2, and leptoskit reads who they are, and their addresses, from Bitbucket's API. Last checked against Atlassian's documentation on 31 October 2026.

## Set it up

1. Run `leptoskit auth provider add bitbucket`. It adds:

   ```toml
   [auth.providers.bitbucket]
   client_id_env = "BITBUCKET_CLIENT_ID"
   client_secret_env = "BITBUCKET_CLIENT_SECRET"
   ```

2. In your Bitbucket workspace's settings, open OAuth consumers and add a consumer with the "Account: Email" and "Account: Read" permissions, and the Callback URL `https://myapp.com/auth/callback/bitbucket`. A consumer takes one callback URL, so add a second consumer for development, with `http://localhost:3000/auth/callback/bitbucket`.
3. Set `BITBUCKET_CLIENT_ID` (the consumer's key) and `BITBUCKET_CLIENT_SECRET`.
4. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

The person's Bitbucket UUID, which never changes, their display name, nickname, and avatar, and their primary email address from Bitbucket's list of their addresses, with its `is_confirmed` flag. leptoskit asks for the `account` and `email` scopes. A confirmed address is stored as the account's; an unconfirmed one is kept on the connected account for display only.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
