---
"release": minor
---

# Two-step sign-in, and sign-in in native guests

An emailed code proves only that someone can read the inbox. Accounts can now turn on an authenticator app, so whoever gets into a person's email still can't get into their account, and sensitive actions can ask for a fresh code. It works the same in the browser and in native apps.

- **Authenticator apps (TOTP):** `enroll_totp()` and `confirm_totp()` turn one on with a code from it, so a mistyped secret never locks anyone out. Once it's on, `verify_code()` answers `SignInStep::SecondFactor`, and `complete_sign_in()` finishes with the app's code. Codes work once, and the secret is encrypted in the database with AES-256-GCM-SIV under `AUTH_ENCRYPTION_KEYS`.
- **Recovery codes:** ten, shown once when the authenticator is turned on, each signing in once if the phone is lost. Using one signs out every other device and emails the account. `regenerate_recovery_codes()` replaces them.
- **Step-up:** `auth::require_step_up(auth::STEP_UP_WINDOW)` in a server function asks for a code from the last ten minutes; the client answers `step_up_required` with `step_up()` and tries again. Stepping up replaces the session's token.
- **Screens:** `leptoskit generate auth` and `leptoskit new --auth` write a two-step verification screen and a `/security` route, and the sign-in screen asks for the second step when an account has one.
- **Native guests:** a native app built with `--auth` now loads and runs its sign-in screens. Browser-only timers no longer reach native builds, the native runtime tells guests the time through a new optional `leptoskit_clock` export, wasm-bindgen glue that only a browser could call links to a trap that names it instead of stopping the app from loading, and the `http` and `secure-store` plugins are granted and added to the iOS, Android, and macOS hosts. The Linux, Windows, and terminal runners can't host plugins yet: they now build without them, with a warning, instead of refusing to build.
