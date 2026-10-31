---
"release": minor
---

# Passkeys, required two-step, and account lockout

`leptoskit::auth` signs people in with passkeys on the web, and can require two-step verification.

- **Passkeys** (the `auth-passkeys` feature on your server, plus `[auth] origin`): `add_passkey()`, `passkey_sign_in()` from a button or from the email field's suggestions (`PasskeyMode::Autofill`, with the field's new `.autocomplete("username webauthn")`), `complete_sign_in_with_passkey()` as the second step, `step_up_with_passkey()`, and `use_passkeys()`, `rename_passkey()`, and `remove_passkey()`. A passkey that verifies its user signs in at `aal2` on its own. The functions exist on every platform; native apps answer `unsupported` until their passkey plugin arrives.
- **Required two-step verification:** `[auth] mfa = "required"`, or `required_for = ["Admin"]` with `AuthBuilder::roles::<User>()`. Until someone sets up a factor, their session can only do that; everything else answers `mfa_required`.
- **Account lockout:** after 20 wrong second factors in a day, sign-in is locked for an hour and the owner is emailed. `Auth::unlock` lifts it, and `Auth::reset_mfa` removes an account's authenticator, recovery codes, and passkeys for someone locked out of their own.
- **QR codes:** `client::qr_code(&setup.uri)` draws the authenticator setup link as a `Vector` path or SVG, made on the device.
- **`leptoskit auth rotate-keys`** re-encrypts stored secrets under the newest key, and **`leptoskit auth doctor`** checks `[auth]` without printing secrets; `--release` fails on plain `http` origins and the development mailbox.
- **The generated screens** gain passkeys, the QR code, and a "Security" tab.
