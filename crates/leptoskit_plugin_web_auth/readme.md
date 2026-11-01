<h1 align="center">leptoskit_plugin_web_auth</h1>

<p align="center">
  <strong>leptoskit_plugin_web_auth</strong> is the leptoskit plugin for sign-in sessions in the system browser - ASWebAuthenticationSession on Apple platforms, Auth Tab on Android, and a loopback redirect on desktop.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_web_auth"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_web_auth"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_web_auth.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_web_auth"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_web_auth/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Sign-in sessions in the system browser, never a web view inside the app, as [RFC 8252](https://www.rfc-editor.org/rfc/rfc8252) asks of native apps. The sign-in client in `leptoskit_auth` uses it for third-party providers; apps rarely call it directly.

Enable the `guest` feature for an interpreted Wasm app and grant `web-auth` in `native.plugins`.

| Platform      | Browser                                       | Comes back through                                                       |
| ------------- | --------------------------------------------- | ------------------------------------------------------------------------ |
| iOS, macOS    | `ASWebAuthenticationSession`                  | `<bundle id, lowercased>:/auth/native/return`, to this session only      |
| Android       | Auth Tab, with Chrome 137 or later as default | `<package name, lowercased>:/auth/native/return`, as the activity result |
| Windows/Linux | The default browser, through `open`           | `http://127.0.0.1:<port>/auth/native/return`, bound for the session      |
| Web           | Unavailable: the page navigates itself        |                                                                          |

Android browsers without Auth Tab answer `Unavailable`; there is no Custom Tabs fallback yet. On Windows and Linux, register `desktop::WebAuth` yourself on a worker: waiting for the browser blocks for as long as the person takes, up to ten minutes. The listener answers only `GET` on the return path, with a page that says to go back to the app, and `404` to anything else.

Wire operations: method 0 (`BEGIN`) takes an empty payload and answers a little-endian `u32` session ID followed by the UTF-8 return address. Method 1 (`AUTHENTICATE`) takes the session ID followed by the provider's page, which must be `https` (or `http` on this device), and answers the UTF-8 address the browser came back to, which must be the session's return address with only a query added. One session runs at a time; a call for another session is `InvalidCall`, closing the browser is `Cancelled`, and dropping the call closes the browser session. Payloads are revalidated in Swift and Kotlin.

Nothing that comes back is a credential on its own: the sign-in server finishes the flow only with a binding the app kept in memory, so an answer another app intercepts, or a link someone else started, signs no one in.

API references: [Expo AuthSession](https://docs.expo.dev/versions/latest/sdk/auth-session/), [ASWebAuthenticationSession](https://developer.apple.com/documentation/authenticationservices/aswebauthenticationsession), and [Auth Tab](https://developer.chrome.com/docs/android/custom-tabs/guide-auth-tab). This is an idiomatic Rust API over native services, not a source or API compatibility claim.

## How it fits

Add `leptoskit_plugin_web_auth` as a direct dependency and select the `web-auth` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Guests enable the `guest` feature and reuse `guest::WebAuth` rather than resolving the plugin on every call. It follows RFC 8252: the provider page never loads in a web view inside the app.

## Use it

```rust,ignore
use leptoskit_plugin_web_auth::guest::WebAuth;

let web_auth = WebAuth::new()?;
let session = web_auth.begin().await?;          // where the browser comes back to
// …start the provider flow on the server with `session.return_to`…
let returned = web_auth.authenticate(&session, &authorization_url).await?;
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_web_auth)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
