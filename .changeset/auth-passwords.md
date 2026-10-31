---
"release": minor
---

# Passwords, for apps that want them

`[auth.passwords] enabled = true` (with the `auth-passwords` feature on your server) lets people set a password and sign in with their email address and it. Passwords stay off by default.

- **Rules from NIST SP 800-63B:** at least 15 characters by default (`min_length`, down to 8 when everyone has two-step verification), at most 256, no rules about kinds of characters, and no common passwords or ones close to the email address or the app's name.
- **An optional breach check** against Have I Been Pwned (`breach_check = true`), which sends only the first 5 characters of the password's SHA-1 hash.
- **Argon2id** at OWASP's settings, re-hashed at sign-in when the settings change.
- **Resets** take an emailed code and a new password, end every session, and wait for the second factor when the account has one.
- **No lockouts by guessing:** after too many wrong passwords for an address, only password sign-in pauses for an hour; emailed codes and passkeys keep working, and the owner is emailed.
- `Auth::sign_in_with_password`, `set_password`, `remove_password`, `reset_password`, and `password_settings`, with routes under `/auth/password`. `MfaStatus` gained `password`. `leptoskit auth doctor` warns when `min_length` is under 15 without required two-step verification.
