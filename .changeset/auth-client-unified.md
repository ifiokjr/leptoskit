---
"release": minor
---

# One sign-in client for every platform

`leptoskit::auth::client` now works the same in the browser and in native apps on iOS, Android, macOS, Windows, and Linux: the same functions, the same arguments, and the same behaviour. A sign-in screen and a devices screen are written once, with no platform checks.

- **`configure(ClientConfig::new().origin(…).device_name(…))`** tells the client where the server is, once at startup, on every platform. On the web the origin defaults to the page's own.
- **`verify_code()`** takes `EmailCode { email, code }`. The client decides how the device keeps its session.
- **In the browser**, the session stays in the `HttpOnly` cookie, and the client keeps nothing scripts could read.
- **In a native app**, the client keeps the access token in memory and the refresh token in the secure store. It renews the access token when it runs out, one renewal at a time, and signs the app out when the session has ended elsewhere.
- **During server rendering**, `use_auth()` and `use_devices()` read the request's session directly.
- **`revoke_other_devices()` and `sign_out_everywhere_mutation()`** now return `()`.
- **New error codes:** `config` when the app isn't set up to reach the server, and `storage` when the secure store fails.

The client uses the `/auth` routes on every platform, and the facade's `web` and `guest` features choose how it reaches the server.
