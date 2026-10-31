# LeptosKit Passkeys

Passkeys and Sign in with Apple with the platform's own credential sheet. The sign-in client in `leptoskit_auth` uses it; apps rarely call it directly.

```rust,ignore
use leptoskit_plugin_passkeys::guest::Passkeys;

let passkeys = Passkeys::new()?;
let registration = passkeys.create(&creation_options_json).await?;
let assertion = passkeys.get(&request_options_json).await?;
let apple = passkeys.apple_id(&nonce).await?;   // ID token for the app's bundle ID
```

Enable the `guest` feature for an interpreted Wasm app and grant `passkeys` in `native.plugins`.

| Operation          | iOS, macOS                                           | Android            | Windows, Linux, web |
| ------------------ | ---------------------------------------------------- | ------------------ | ------------------- |
| Create a passkey   | `ASAuthorizationPlatformPublicKeyCredentialProvider` | Credential Manager | Unavailable         |
| Sign with one      | `ASAuthorizationPlatformPublicKeyCredentialProvider` | Credential Manager | Unavailable         |
| Sign in with Apple | `ASAuthorizationAppleIDProvider`                     | Unavailable        | Unavailable         |

The relying party must vouch for the app. On Apple platforms its `apple-app-site-association` lists the app under `webcredentials`, and the app has the associated domain `webcredentials:<host>`. On Android its `assetlinks.json` grants `delegate_permission/common.get_login_creds` to the app's signing certificate. `leptoskit::server::app_associations` serves both from `[native]`. User verification is always required. Passkeys are not offered in text-field autofill.

Wire operations: method 0 takes WebAuthn's `PublicKeyCredentialCreationOptionsJSON` and answers `RegistrationResponseJSON`; method 1 takes `PublicKeyCredentialRequestOptionsJSON` and answers `AuthenticationResponseJSON`; both are UTF-8 JSON objects of at most 64 KiB, and options must carry a `challenge`. Method 2 takes a nonce of 1 to 128 printable ASCII characters and answers `{"id_token", "given_name"?, "family_name"?}`. Closing the sheet is `Cancelled`; a platform without the operation, or Credential Manager without a provider, is `Unavailable`; other failures are `Native`. One request runs at a time.

API references: [Expo AppleAuthentication](https://docs.expo.dev/versions/latest/sdk/apple-authentication/), [ASAuthorization](https://developer.apple.com/documentation/authenticationservices/asauthorization), and [Credential Manager](https://developer.android.com/identity/sign-in/credential-manager). This is an idiomatic Rust API over native services, not a source or API compatibility claim.
