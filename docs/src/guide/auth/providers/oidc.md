# Any OpenID Connect provider

Lets people sign in through any OpenID Connect provider: Okta, Auth0, Keycloak, Authentik, Zitadel, your company's own Microsoft Entra tenant, or another leptoskit app. Give it an ID of your own, such as `company`, which names it in routes and in `leptoskit.toml`. An ID that names a built-in provider, such as `slack`, uses that provider's definition; add `kind = "oidc"` to use the issuer's discovery document instead.

## Set it up

1. Run `leptoskit auth provider add company --issuer https://sso.example.com`. It adds:

   ```toml
   [auth.providers.company]
   issuer = "https://sso.example.com"
   client_id_env = "COMPANY_CLIENT_ID"
   client_secret_env = "COMPANY_CLIENT_SECRET"
   ```

   Add `name = "Company SSO"` for its button. Without `client_secret_env`, it's a public client, and PKCE alone protects the exchange.
2. Register a web application with the provider, allowing the redirect URI `https://myapp.com/auth/callback/company`, and `http://localhost:3000/auth/callback/company` for development.
3. Set the client ID and secret variables.
4. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

leptoskit reads the issuer's discovery document, `/.well-known/openid-configuration`, which must name exactly the configured issuer and `https` endpoints, and checks each ID token against the keys it publishes. The issuer must be `https`, or `http` on `localhost` for a provider you run locally. An email address is stored as verified only when the token's `email_verified` says so.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
