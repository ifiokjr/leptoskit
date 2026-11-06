---
"release": minor
---

# Add password fields to the generated sign-in screens

With `[auth.passwords] enabled = true`, `leptoskit generate auth` now writes password fields into the screens. The sign-in screen offers "Sign in with a password" and resets a forgotten password with an emailed code; the security screen sets, changes, and removes the password, with the server's minimum length beside the field. The fields are `PasswordInput`s marked `current-password` or `new-password`, so password managers fill and save them on the web and in native apps.

Turning passwords on later and running `leptoskit generate auth` again adds the fields to screens you haven't changed, and leaves the ones you have alone unless you pass `--force`. Turning them off never takes fields away.

In app code, `leptoskit::auth::client::sign_in_with_password()` takes an `EmailPassword` (the address and password), alongside `EmailCode` for emailed codes.
