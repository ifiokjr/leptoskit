---
"release": minor
---

# Passkeys and provider sign-in in native apps

Native apps now sign in with passkeys and with Google, Apple, Microsoft, GitHub, and any OpenID Connect provider, through the same `leptoskit::auth::client` functions as the web. App code doesn't change.

- **Two new plugins.** `passkeys` uses the system's own sheet: `ASAuthorization` on iOS and macOS, Credential Manager on Android, and Sign in with Apple's own sheet for Apple on Apple devices. `web-auth` signs in with other providers in the system browser, never a web view in the app: `ASWebAuthenticationSession` on iOS and macOS, Auth Tab on Android (Chrome 137 or later), and a loopback address in the default browser for Windows and Linux hosts that register it.
- **The flow.** The provider sends the browser back to your site's `/auth/callback/…`, as for the web, so providers need no new setup. Your site sends the browser on to the app with the provider's answer, and the app finishes at `POST /auth/oauth/finish` with a binding only it holds, so an intercepted answer or a link someone else started signs no one in. `sign_in_with_provider()` resolves once the browser is back, and `provider_outcome()` says how it ended.
- **Your site vouches for its apps.** Set `apple_team_id` and `android_certificates` in `[native]`, and merge `leptoskit::server::app_associations` (or the generated `sign_in::associations()`) into your router: it serves `/.well-known/apple-app-site-association` and `/.well-known/assetlinks.json` from `[native]`. The server then accepts the Android app's passkeys, and Sign in with Apple ID tokens for your bundle ID.
- **Scaffold.** `leptoskit new --auth` and `leptoskit generate auth` grant `passkeys` and `web-auth` alongside `http` and `secure-store`, and the generated server serves the association files.

Native apps don't offer passkeys in a text field's suggestions; the generated Xcode projects don't add the associated-domain or Sign in with Apple entitlements yet; Android has no fallback for browsers without Auth Tab; and Windows and Linux have no native passkeys.
