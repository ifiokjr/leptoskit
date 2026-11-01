<h1 align="center">leptoskit_plugin_passkeys</h1>

<p align="center">
  <strong>leptoskit_plugin_passkeys</strong> is the leptoskit plugin for passkeys and Sign in with Apple through the platform's own credential sheet.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_passkeys"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_passkeys"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_passkeys.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_passkeys"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_passkeys/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Passkeys and Sign in with Apple with the platform's own credential sheet. The sign-in client in `leptoskit_auth` uses it; apps rarely call it directly.

Enable the `guest` feature for an interpreted Wasm app and grant `passkeys` in `native.plugins`.

| Operation          | iOS, macOS                                           | Android            | Windows, Linux, web |
| ------------------ | ---------------------------------------------------- | ------------------ | ------------------- |
| Create a passkey   | `ASAuthorizationPlatformPublicKeyCredentialProvider` | Credential Manager | Unavailable         |
| Sign with one      | `ASAuthorizationPlatformPublicKeyCredentialProvider` | Credential Manager | Unavailable         |
| Sign in with Apple | `ASAuthorizationAppleIDProvider`                     | Unavailable        | Unavailable         |

The relying party must vouch for the app. On Apple platforms its `apple-app-site-association` lists the app under `webcredentials`, and the app has the associated domain `webcredentials:<host>`. On Android its `assetlinks.json` grants `delegate_permission/common.get_login_creds` to the app's signing certificate. `leptoskit::server::app_associations` serves both from `[native]`. User verification is always required. Passkeys are not offered in text-field autofill.

Wire operations: method 0 takes WebAuthn's `PublicKeyCredentialCreationOptionsJSON` and answers `RegistrationResponseJSON`; method 1 takes `PublicKeyCredentialRequestOptionsJSON` and answers `AuthenticationResponseJSON`; both are UTF-8 JSON objects of at most 64 KiB, and options must carry a `challenge`. Method 2 takes a nonce of 1 to 128 printable ASCII characters and answers `{"id_token", "given_name"?, "family_name"?}`. Closing the sheet is `Cancelled`; a platform without the operation, or Credential Manager without a provider, is `Unavailable`; other failures are `Native`. One request runs at a time.

API references: [Expo AppleAuthentication](https://docs.expo.dev/versions/latest/sdk/apple-authentication/), [ASAuthorization](https://developer.apple.com/documentation/authenticationservices/asauthorization), and [Credential Manager](https://developer.android.com/identity/sign-in/credential-manager). This is an idiomatic Rust API over native services, not a source or API compatibility claim.

## How it fits

Add `leptoskit_plugin_passkeys` as a direct dependency and select the `passkeys` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::Passkeys` rather than resolving the plugin on every call. Apple platforms use `ASAuthorization` and Android uses Credential Manager; WebAuthn JSON crosses the bridge unchanged.

## Use it

```rust,ignore
use leptoskit_plugin_passkeys::guest::Passkeys;

let passkeys = Passkeys::new()?;
let registration = passkeys.create(&creation_options_json).await?;
let assertion = passkeys.get(&request_options_json).await?;
let apple = passkeys.apple_id(&nonce).await?;   // ID token for the app's bundle ID
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_passkeys)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
