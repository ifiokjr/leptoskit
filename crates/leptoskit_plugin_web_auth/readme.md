# LeptosKit Web Auth

Sign-in sessions in the system browser, never a web view inside the app, as [RFC 8252](https://www.rfc-editor.org/rfc/rfc8252) asks of native apps. The sign-in client in `leptoskit_auth` uses it for third-party providers; apps rarely call it directly.

```rust,ignore
use leptoskit_plugin_web_auth::guest::WebAuth;

let web_auth = WebAuth::new()?;
let session = web_auth.begin().await?;          // where the browser comes back to
// …start the provider flow on the server with `session.return_to`…
let returned = web_auth.authenticate(&session, &authorization_url).await?;
```

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
