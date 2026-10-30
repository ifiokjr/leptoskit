# Threat model: `leptoskit_auth`

This is the threat model the [auth plan](auth-plan.md) asks for in phase Auth 2. It covers what `leptoskit_auth` builds today: email codes, sessions in the browser and in native apps, authenticator apps (TOTP), recovery codes, and step-up. Passkeys, third-party sign-in, Solana wallets, account merging, and `leptoskit_idp` get their own sections when they land. Update this file in the same pull request as any change to what it describes.

## What we protect

| Asset                           | Where it lives                                                                                                                          | Why it matters                                                          |
| ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| Session tokens                  | Browser: `__Host-session` cookie (`HttpOnly`). Native: access token in memory, refresh token in the secure store. Server: SHA-256 only. | Whoever holds one is the user until it ends.                            |
| Emailed codes                   | The inbox; the server keeps an HMAC bound to purpose and address                                                                        | A code is the first factor.                                             |
| TOTP secrets                    | The person's authenticator app; the server keeps AES-256-GCM-SIV ciphertext bound to the account                                        | A secret produces every future code.                                    |
| Recovery codes                  | Wherever the person saved them; the server keeps HMACs bound to the account                                                             | Each one replaces the authenticator once.                               |
| The second step's token         | Browser: `__Host-second-factor` cookie (`HttpOnly`, 5 minutes). Native: memory. Server: SHA-256.                                        | Proves the first factor happened on this device.                        |
| `AUTH_ENCRYPTION_KEYS`          | The server's environment, never `leptoskit.toml`                                                                                        | Hashes codes and encrypts TOTP secrets; derives separate keys for each. |
| The audit trail                 | `auth.events`                                                                                                                           | How an owner or operator finds out what happened.                       |
| Who policies think is signed in | `SignedIn` in the request's `Context`                                                                                                   | Every table's access policy trusts it.                                  |

## Who we defend against

1. **A remote attacker** with the app's URL: guesses codes, stuffs credentials, enumerates accounts, forges cross-site requests.
2. **A phisher** who gets a person to type a code into the wrong site.
3. **Someone with the person's email**, through a reused password or a compromised mailbox.
4. **Someone with a copy of the database**, from a leaked backup or a read-only SQL injection elsewhere in the app.
5. **Someone with the device for a moment**: an unlocked laptop, a borrowed phone, an open session on a shared computer.
6. **Script injected into the page** (XSS) on the app's origin.
7. **A thief of the phone** that holds the authenticator, or of a native app's secure store.
8. **A network observer** between the device and the server.

Out of scope: someone with the server's environment or memory (they have the keys), a compromised operating system or browser, and a malicious app developer. The app's own server functions are in scope only where `leptoskit_auth` gives them guards.

## Threats and controls

Each row names the control that answers the threat, and the test that holds it. Test paths are relative to `crates/leptoskit_auth`.

### Guessing and enumeration

| Threat                                               | Control                                                                                                                                                           | Tests                                                                                                    |
| ---------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| Guessing an emailed code                             | 40-bit codes, 10 minutes, 5 attempts counted with compare-and-set _before_ the answer is checked, a new code retires old ones; per-IP and per-address limits      | `tests/sign_in.rs`                                                                                       |
| Guessing a TOTP code (1 in 10⁶ per guess, 3 windows) | 5 attempts per second step, 10 per account per 15 minutes from anywhere, 30 per IP; each failure is an audit event                                                | `tests/mfa.rs`: `the_second_step_allows_five_attempts`, `second_factor_attempts_are_limited_per_account` |
| Guessing a recovery code                             | 50 bits each, the same limits as TOTP                                                                                                                             | `tests/mfa.rs`                                                                                           |
| Finding out which addresses have accounts            | Sending a code answers the same for every valid address; a wrong code, an expired one, and no code fail alike; the code hash is computed even without a challenge | `tests/sign_in.rs`, `tests/http.rs`                                                                      |
| Finding out which accounts have two-step on          | Only after a correct emailed code, which already proves the inbox                                                                                                 | by design                                                                                                |
| Timing differences                                   | `subtle` comparisons for every code and hash; every unused recovery code is checked, not the first match; all three TOTP windows are computed                     | `src/totp.rs`, `src/secrets.rs` unit tests                                                               |

**Gap:** an account isn't locked after repeated failures (`accounts.locked_until` exists but nothing sets it yet), and the owner isn't emailed about failed second factors. Rate limits bound the damage; locking with a notification is planned with passwords (Auth 5).

### Replay

| Threat                                         | Control                                                                                                                               | Tests                                                               |
| ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| Reusing a TOTP code (shoulder-surfed, phished) | `totp.last_used_step`: a code's 30-second step is claimed with compare-and-set, so it works once, even across two concurrent requests | `signing_in_asks_for_the_second_factor_and_refuses_a_replayed_code` |
| Reusing a recovery code                        | `used_at` claimed with compare-and-set                                                                                                | `a_recovery_code_signs_in_once_and_ends_the_other_sessions`         |
| Reusing the second step                        | Single use, 5 minutes, retired after its last attempt or by a newer step for the same account                                         | `tests/mfa.rs`, `tests/http.rs`                                     |
| Reusing a refresh token                        | Rotation on every use; a second use after a 30-second grace ends the session and records `refresh_reused`                             | `src/sessions.rs` unit tests, `src/client/tests.rs`                 |

### Phishing

TOTP and emailed codes are **not** phishing-resistant: a site that relays them in real time gets a session. The controls narrow the window (codes work once, steps expire in 5 minutes) and the audit trail records the sign-in with its IP address. Passkeys, which are bound to the origin, are the answer; they arrive in the next layer of Auth 2, and native passkeys in Auth 6. Until then, NIST SP 800-63B rates TOTP as AAL2 but not phishing-resistant, and so do we.

### Stolen database

| What leaks       | Why it doesn't sign anyone in                                                                                                                                                                                               |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Session rows     | Only SHA-256 of 256-bit tokens. Not reversible, not guessable.                                                                                                                                                              |
| Challenge rows   | Emailed codes as HMAC-SHA-256 under the server's keys; second steps as SHA-256 of 256-bit tokens.                                                                                                                           |
| `totp` rows      | AES-256-GCM-SIV under a key derived from `AUTH_ENCRYPTION_KEYS`, with the account in the associated data: copying a ciphertext to another account's row doesn't decrypt. GCM-SIV stays safe if a random nonce ever repeats. |
| `recovery_codes` | HMAC-SHA-256 bound to the account.                                                                                                                                                                                          |
| `events`         | IP addresses and user agents, never credentials. They are personal data: keep backups as you keep the database.                                                                                                             |

A database _write_ (SQL injection with write access) is out of scope: an attacker who can write `auth.sessions` can insert a session for any token they choose.

### Someone with the email

The email address is the only first factor today, so whoever controls the inbox controls an account without two-step verification. With it on, they also need the authenticator or a recovery code. To stop them turning two-step _on_ to lock the owner out:

- turning it on needs a sign-in within the last 10 minutes (`reauthenticate` otherwise), or step-up when a factor exists;
- turning it on, turning it off, and using a recovery code each email the account's primary address.

**Gap:** someone who signs in through the inbox and turns two-step on within 10 minutes locks the owner out until an operator steps in. The email tells the owner; recovery needs an admin action today (`Auth::disable` then support). An admin "reset two-step" is planned with the admin tools.

### Someone at the device

| Threat                                    | Control                                                                                                                         |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| Turning two-step off, or making new codes | Step-up: a code from the authenticator within the last 10 minutes                                                               |
| Turning two-step on to lock the owner out | A sign-in within the last 10 minutes                                                                                            |
| Keeping access after the owner notices    | "Sign out everywhere", revoking one device, and turning two-step off all end the other sessions; using a recovery code does too |
| A token copied before a privilege change  | Confirming an authenticator and stepping up replace the session's token in place; the old one stops working at once             |

The app's own sensitive actions get the same protection through `auth::require_step_up(auth::STEP_UP_WINDOW)` in server functions and `SignedIn::require_step_up` in handlers.

### Script on the page (XSS)

- The session cookie and the second step's cookie are `HttpOnly`. The browser client keeps nothing in script-readable memory or storage: `fetch` is `same-origin` with `credentials: "same-origin"`.
- Injected script can still _act_ as the person while the page is open: it can call every route the page can. Step-up limits what it can change without a code; it can't read the TOTP secret after setup, recovery codes after they're shown, or any token.
- During setup, the TOTP secret and new recovery codes are in the page's memory, because the person has to see them. A page with injected script at that moment leaks them. Content Security Policy is the app's defence there.

### Cross-site requests

State-changing requests must carry the configured `Origin`, or `Sec-Fetch-Site: same-origin`/`none`; a request with neither is accepted only if it carries neither the session cookie nor the second step's cookie (how native apps and scripts call). Cookies are `SameSite=Lax`. A cross-site page can't complete someone else's second step: it has neither the cookie nor the code. Tests: `tests/http.rs`.

### Native apps

| Threat                                | Control                                                                                                                                                 |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Reading tokens from app storage       | The refresh token is in the platform's secure store (`leptoskit.auth`/`refresh_token`); the access token and the second step's token are only in memory |
| A copied refresh token                | Single use with reuse detection; the session ends for every holder                                                                                      |
| The guest reaching other hosts        | The HTTP plugin only reaches origins listed in `[native.plugin-options.http]`; the guest can't change them                                              |
| An over-the-air update leaking tokens | Updates are signed; the token never leaves the guest except to the listed origins                                                                       |

**Gap:** the Linux, Windows, and terminal runners can't host the `http` and `secure-store` plugins yet, so sign-in answers `config` there. When they can, the refresh token must go to the platform credential store (Credential Manager, Secret Service), never a file.

**Gap:** the generated scaffold allows `http://localhost:3000` and `http://10.0.2.2:3000` for development. A release build that still lists them sends tokens over plain HTTP to whatever answers on the device's loopback. `lk auth doctor` (Auth 3) should refuse `http` origins in release builds.

### Keys

- One environment variable, several keys, newest first. The newest hashes and encrypts; any listed key verifies and decrypts, so rotation never breaks codes in flight or stored secrets.
- Hashing and encryption use different keys derived from each configured key (`HMAC(key, "leptoskit-auth encryption v1")` for AES), so no key is used for two algorithms.
- A sealed secret names its key ID. Removing a key that still encrypts a TOTP secret makes that account's second factor fail with a server error (`a stored secret was encrypted with key …`), naming the key and never the value. Recovery codes hashed under a removed key stop matching.
- **Gap:** `lk auth rotate-keys`, which re-encrypts stored secrets under the newest key in the background, isn't built yet. Until then, keep old keys listed.

### Logs and errors

`Debug` for `SessionToken`, `Tokens`, `RefreshTokens`, `EmailCode`, `SecondFactor`, `SecondFactorCode`, `FactorCode`, `TotpSetup`, `RecoveryCodes`, `Keys`, and the TOTP secret prints no secret. HTTP errors never carry database, mailer, or configuration details. Notification emails are best effort: a mailer failure doesn't undo the change, which the audit trail already holds.

## Assurance levels

| Session got there by                          | `aal`  | `mfa_at`        |
| --------------------------------------------- | ------ | --------------- |
| An emailed code, no second factor on          | `aal1` | none            |
| An emailed code, then TOTP or a recovery code | `aal2` | the second step |
| Confirming an authenticator                   | `aal2` | then            |
| Step-up                                       | `aal2` | then            |

An emailed code never counts as a second factor, since email is the first. Step-up looks at `mfa_at`, not only `aal`, so an `aal2` session from yesterday still has to step up.

## What's tested where

- `src/totp.rs`: RFC 4226 and RFC 6238 test vectors, replay and skew, RFC 4648 base32 vectors, `otpauth://` encoding.
- `src/secrets.rs`: sealing round trips, wrong context, tampering, truncation, rotation, a dropped key.
- `src/mfa.rs`: recovery code format, step-up windows.
- `tests/mfa.rs` (SQLite and Postgres): enrolment, re-authentication, the second step, attempts, replay, recovery codes, step-up, turning off, rate limits, audit events, notifications, nothing stored in the clear.
- `tests/http.rs`: the routes, cookies, the second step's cookie lifecycle, bearer flows, error codes.
- `src/client/tests.rs`: the same two-step scenario through the browser's cookie jar and the native token keeper.
- `crates/leptoskit_cli/tests/db.rs` (ignored, end to end): the scaffold signs in over HTTP, and its native guest loads in the interpreter.

## Open items

- Passkeys (web, with conditional UI and management), then native passkeys (Auth 6).
- Account lockout with an email after repeated failures.
- An admin reset for two-step verification.
- `lk auth rotate-keys` and `lk auth doctor`.
- Property tests and fuzzing for cookie, token, and code parsing, per the plan's risks section.
- An external review before the first stable release.
