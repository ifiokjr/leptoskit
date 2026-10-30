---
"release": minor
---

# Native sign-in tokens and a list of signed-in devices

Native apps have no cookies, so a single long-lived token was all they had. Now they get two:

- **An access token** for each request, which works for 15 minutes (`[auth] access_minutes`, up to 60).
- **A refresh token** for the platform's secure store. It trades for a new pair at `POST /auth/refresh` and works once. If a used one turns up again after a 30-second grace, the session ends: someone else has a copy.

Browsers keep their `HttpOnly` cookie, which scripts can't read. Every token is random rather than signed, and checked on every request, so ending a session takes effect at once.

People can also see where they're signed in and end any of it:

- **`GET /auth/sessions`** lists each device, marking the current one.
- **`POST /auth/sessions/revoke-all`** and **`auth.sign_out_everywhere(account)`** end every session, the current one included.
- **The headless client** adds `use_devices()`, `revoke_device()`, `revoke_other_devices()` and `sign_out_everywhere_mutation()`.
- **Device names:** a native app can name its device, such as "Ada's iPhone", when it signs in.

`SignedInResponse.token` is now `tokens`.
