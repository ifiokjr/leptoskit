# Microsoft

Lets people sign in with a Microsoft work or school account (Microsoft Entra ID), or a personal Microsoft account. Last checked against Microsoft's documentation on 30 October 2026.

## Set it up

1. Run `leptoskit auth provider add microsoft`. It adds:

   ```toml
   [auth.providers.microsoft]
   client_id_env = "MICROSOFT_CLIENT_ID"
   client_secret_env = "MICROSOFT_CLIENT_SECRET"
   tenant = "common"
   ```

2. In the [Microsoft Entra admin center](https://entra.microsoft.com), under App registrations, create a registration with a "Web" redirect URI of `https://myapp.com/auth/callback/microsoft`, and `http://localhost:3000/auth/callback/microsoft` for development.
3. Under Certificates & secrets, create a client secret.
4. Set `MICROSOFT_CLIENT_ID` (the application ID) and `MICROSOFT_CLIENT_SECRET`.
5. Choose `tenant`: `common` for any account, `organizations` for work and school accounts only, `consumers` for personal accounts only, or your tenant's ID for your own organization only.
6. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

A stable ID for the person and their name. The ID token's issuer must be the person's own tenant, and the tenant must be one `tenant` allows.

Microsoft's email claim is never trusted: an organization's administrator can set it to any address, which has been used to take over accounts in apps that matched on it. leptoskit keeps it on the connected account for display, but never stores it as a verified address or uses it to find another account.

## Platforms

Signing in with Microsoft works on the web. Native apps answer `unsupported` until the system-browser plugin arrives.
