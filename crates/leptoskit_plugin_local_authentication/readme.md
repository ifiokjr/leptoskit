<h1 align="center">leptoskit_plugin_local_authentication</h1>

<p align="center">
  <strong>leptoskit_plugin_local_authentication</strong> is the leptoskit plugin for local device-owner authentication through Apple `LAContext` and Android `BiometricPrompt`.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_local_authentication"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_local_authentication"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_local_authentication.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_local_authentication"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_local_authentication/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Local device-owner authentication through Apple `LAContext` and Android `BiometricPrompt`. The plugin checks capabilities without prompting and returns explicit terminal outcomes from the operating system. It does not identify an account or authorize a server request, and it is not a cryptographic key operation.

Add the crate as a direct dependency and `"local-authentication"` to `native.plugins`. Android registration needs an Activity context and declares `android.permission.USE_BIOMETRIC`. Apple registers the same Swift implementation on iOS and macOS. Android API 30+ is required for explicit strong-biometric and device-credential policies; earlier Android releases return unavailable. Windows, Linux and browsers do not implement this native authentication contract. Browser calls explicitly return `Unavailable`; WebAuthn is not substituted.

For an iOS app that uses Face ID, add an app-owned purpose string to `hosts/ios/Info.plist`:

```xml
<key>NSFaceIDUsageDescription</key>
<string>Use Face ID to unlock your private notes.</string>
```

The plugin never generates a generic purpose string. An authentication request on Face ID hardware with a missing or blank description fails with `InvalidArgument`. It does not silently switch to passcode authentication. Android missing manifest permission is reported by readiness and rejects authentication with `PermissionDenied`.

`BiometricOnly` permits only biometrics; Android uses Class 3 (`BIOMETRIC_STRONG`). `BiometricOrCredential` explicitly permits the operating system's device PIN, pattern or password. No custom credential collection UI exists. Capability modality flags describe hardware, not enrollment or security strength. Android can report face hardware while the strong-biometric policy remains unavailable. A readiness response is not evidence of authentication.

One prompt may be active. Overlap returns `Busy`; explicit `cancel` completes the active request with `AppCancelled`. Dropping a guest call or closing the host cancels native work. Callbacks are delivered on the host UI thread and checked against the original request token. Late callbacks cannot complete a replacement request. Apple creates a fresh context with authentication reuse disabled for each request. Android non-matching scans remain retryable in the OS prompt until a terminal callback arrives.

Authentication starts only while the Apple app is active or the Android Activity's content is visible; it need not have window focus yet, so an app can authenticate at launch. Apple cancels pending work when iOS enters the background or macOS hides the app. Android closes the plugin when its owning Activity is destroyed; ordinary pause/stop events are left to the OS prompt because Android's full-screen device-credential UI can cause those events. Focus loss alone never cancels an existing prompt.

Only an OS success callback produces `Authenticated`. Android exposes whether a biometric or device credential succeeded. Apple reports biometric provenance for biometrics-only policy and unknown method for its combined policy, because the OS does not disclose which authenticator satisfied that policy. The guest decoder rejects credential or unknown-method success under a biometrics-only request.

The current subset covers capability checks, explicit authentication policy, terminal results and cancellation from [Expo LocalAuthentication](https://docs.expo.dev/versions/latest/sdk/local-authentication/) and [Tauri Biometric](https://v2.tauri.app/plugin/biometric/). Enrollment changes, weak Android biometrics, biometric-gated key storage, cached authentication and platform-specific prompt layout options are outside this implementation.

Tests cover malformed wire messages, credential-downgrade rejection, capability queries, pre-cancelled requests, missing permission and closed handlers. Native tests never show an authentication prompt. Successful biometric or device-PIN interaction on physical hardware has not been verified.

## How it fits

Add `leptoskit_plugin_local_authentication` as a direct dependency and select the `local-authentication` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::LocalAuthentication` rather than resolving the plugin on every call. This is not account identity or server authorization; the plugin reports explicit terminal outcomes and never invents a success.

## Use it

```rust,ignore
use leptoskit_plugin_local_authentication::{guest::LocalAuthentication, Policy, Request};
let authentication = LocalAuthentication::new()?;
let status = authentication.capabilities(Policy::BiometricOnly).await?;
let request = Request::new(Policy::BiometricOnly, "Unlock your private notes")?;
let outcome = authentication.authenticate(&request).await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_local_authentication)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
