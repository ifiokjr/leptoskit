# GitHub

Lets people sign in with their GitHub account. GitHub uses OAuth 2 rather than OpenID Connect, so leptoskit reads who they are from GitHub's API. Last checked against GitHub's documentation on 30 October 2026.

## Set it up

1. Run `leptoskit auth provider add github`. It adds:

   ```toml
   [auth.providers.github]
   client_id_env = "GITHUB_CLIENT_ID"
   client_secret_env = "GITHUB_CLIENT_SECRET"
   ```

2. In [GitHub's developer settings](https://github.com/settings/developers), create a new OAuth App with the authorization callback URL `https://myapp.com/auth/callback/github`. An OAuth App takes one callback URL, so create a second app for development, with `http://localhost:3000/auth/callback/github`.
3. Generate a client secret.
4. Set `GITHUB_CLIENT_ID` and `GITHUB_CLIENT_SECRET`.
5. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

The person's numeric GitHub ID, which never changes, their username and name, and their primary email address, with GitHub's `verified` flag. leptoskit asks for the `read:user` and `user:email` scopes, since a person's primary address may be private.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
