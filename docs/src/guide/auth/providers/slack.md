# Slack

Lets people sign in with their Slack account, through Sign in with Slack (OpenID Connect). Last checked against Slack's documentation on 31 October 2026.

## Set it up

1. Run `leptoskit auth provider add slack`. It adds:

   ```toml
   [auth.providers.slack]
   client_id_env = "SLACK_CLIENT_ID"
   client_secret_env = "SLACK_CLIENT_SECRET"
   ```

2. In [Slack's app settings](https://api.slack.com/apps), create an app, and under OAuth & Permissions add the user scopes `openid`, `email`, and `profile`.
3. Add the Redirect URLs `https://myapp.com/auth/callback/slack`, and `http://localhost:3000/auth/callback/slack` for development.
4. Set `SLACK_CLIENT_ID` and `SLACK_CLIENT_SECRET`.
5. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

Slack's ID token: the person's user ID, their workspace, their name and picture, and their email address with Slack's `email_verified` flag.

A Slack account belongs to one workspace, so leptoskit identifies the person by the workspace and the user together. Someone who signs in from two workspaces has two connected accounts, which they can connect or merge into one account themselves.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
