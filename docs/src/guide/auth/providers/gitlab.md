# GitLab

Lets people sign in with their account on GitLab.com, or on your own self-managed GitLab. Last checked against GitLab's documentation on 31 October 2026.

## Set it up

1. Run `leptoskit auth provider add gitlab`, or for a self-managed GitLab, `leptoskit auth provider add gitlab --issuer https://gitlab.example.com`. It adds:

   ```toml
   [auth.providers.gitlab]
   issuer = "https://gitlab.example.com" # only for a self-managed GitLab
   client_id_env = "GITLAB_CLIENT_ID"
   client_secret_env = "GITLAB_CLIENT_SECRET"
   ```

2. In GitLab, open User settings → Applications (or a group's or the instance's Applications page), and add a confidential application with the scopes `openid`, `profile`, and `email`.
3. Set its Redirect URIs to `https://myapp.com/auth/callback/gitlab`, and `http://localhost:3000/auth/callback/gitlab` for development.
4. Set `GITLAB_CLIENT_ID` (the application ID) and `GITLAB_CLIENT_SECRET`. Without `client_secret_env`, it's a public client, and PKCE alone protects the exchange.
5. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

The address must be `https`, or `http` on `localhost` for a GitLab you run locally.

## What sign-in gets

GitLab's OpenID Connect ID token: the person's GitLab user ID, their name, username, and picture, and their email address with GitLab's `email_verified` flag. Two GitLab instances are two different providers: give each its own ID, such as `[auth.providers.work-gitlab]` with `kind = "gitlab"`.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
