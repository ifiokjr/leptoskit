---
"release": minor
---

# Passwords from the app, on every platform

The headless client gained `password_settings()`/`use_password_settings()`, `sign_in_with_password()` (a `PasswordInput { email, password }`, answering a `SignInStep` like `verify_code()`), `set_password()`, `remove_password()`, and `reset_password()` (a `PasswordReset { email, code, password }`). They're the same in the browser, where the session stays in its `HttpOnly` cookie, and in native apps, which keep their tokens in the secure store. `PasswordInput` and `PasswordReset` leave the password out of `Debug`.

`leptoskit generate auth` turns on `auth-passwords` for the server's build when `[auth.passwords] enabled` is set. Its screens don't have password fields yet, because `TextInput` can't hide what's typed in native apps.
