# LinkedIn

Lets people sign in with their LinkedIn account, through "Sign In with LinkedIn using OpenID Connect" rather than LinkedIn's retired profile API. Last checked against LinkedIn's documentation on 31 October 2026.

## Set it up

1. Run `leptoskit auth provider add linkedin`. It adds:

   ```toml
   [auth.providers.linkedin]
   client_id_env = "LINKEDIN_CLIENT_ID"
   client_secret_env = "LINKEDIN_CLIENT_SECRET"
   ```

2. In [LinkedIn's developer portal](https://www.linkedin.com/developers/apps), create an app and add the "Sign In with LinkedIn using OpenID Connect" product.
3. On its Auth tab, add the Authorized redirect URLs `https://myapp.com/auth/callback/linkedin`, and `http://localhost:3000/auth/callback/linkedin` for development.
4. Set `LINKEDIN_CLIENT_ID` and `LINKEDIN_CLIENT_SECRET`.
5. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

LinkedIn's ID token: a stable ID for the person, their name and picture, and their email address with LinkedIn's `email_verified` flag. leptoskit asks for the `openid`, `profile`, and `email` scopes.

LinkedIn's ID tokens leave out the nonce that other providers echo back. leptoskit accepts that for LinkedIn alone: the token comes straight from LinkedIn's token endpoint, for a code that only finishes in the browser that started the sign-in. A token that does carry a nonce must carry the right one.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
