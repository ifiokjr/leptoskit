# Threat model: `leptoskit_auth`

This is the threat model the [auth plan](auth-plan.md) asks for in phase Auth 2. It covers what `leptoskit_auth` builds today: email codes, sessions in the browser and in native apps, authenticator apps (TOTP), recovery codes, step-up, required two-step verification and lockout, passkeys on the web, third-party sign-in (OpenID Connect and OAuth 2) on the web, and linking, unlinking, and merging accounts ("move only the login"). Solana wallets, merging with data, native passkeys and system-browser sign-in, and `leptoskit_idp` get their own sections when they land. Update this file in the same pull request as any change to what it describes.

## What we protect

| Asset                           | Where it lives                                                                                                                          | Why it matters                                                            |
| ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| Session tokens                  | Browser: `__Host-session` cookie (`HttpOnly`). Native: access token in memory, refresh token in the secure store. Server: SHA-256 only. | Whoever holds one is the user until it ends.                              |
| Emailed codes                   | The inbox; the server keeps an HMAC bound to purpose and address                                                                        | A code is the first factor.                                               |
| TOTP secrets                    | The person's authenticator app; the server keeps AES-256-GCM-SIV ciphertext bound to the account                                        | A secret produces every future code.                                      |
| Recovery codes                  | Wherever the person saved them; the server keeps HMACs bound to the account                                                             | Each one replaces the authenticator once.                                 |
| The second step's token         | Browser: `__Host-second-factor` cookie (`HttpOnly`, 5 minutes). Native: memory. Server: SHA-256.                                        | Proves the first factor happened on this device.                          |
| `AUTH_ENCRYPTION_KEYS`          | The server's environment, never `leptoskit.toml`                                                                                        | Hashes codes and encrypts TOTP secrets; derives separate keys for each.   |
| The audit trail                 | `auth.events`                                                                                                                           | How an owner or operator finds out what happened.                         |
| Passkey public keys             | `auth.passkeys`: public key, counter, backup flag, as `webauthn-rs` serializes them                                                     | Which keys sign an account in; not secret, but must not be swapped.       |
| Ceremony state                  | `auth.challenges.state`: sealed (AES-256-GCM-SIV) and bound to its purpose, subject, and row; 5 minutes (passkeys) or 10 (providers)    | The challenge a passkey signs; a provider flow's PKCE verifier and nonce. |
| Ceremony binding                | Browser: `__Host-passkey` and `__Host-provider` cookies (`HttpOnly`). Native: the body. Server: SHA-256.                                | Ties a ceremony to the browser that started it (login CSRF).              |
| Provider credentials            | Environment variables (`client_secret_env`, Apple's `.p8` in `private_key_env`); never `leptoskit.toml`                                 | Lets anyone holding them impersonate the app at the provider.             |
| Who policies think is signed in | `SignedIn` in the request's `Context`                                                                                                   | Every table's access policy trusts it.                                    |

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

**Lockout.** Wrong second factors (TOTP, recovery codes, and passkeys, at the second step, at step-up, and for the other account in a merge) count per account through the rate limiter: 20 a day by default. The first failure past the limit sets `accounts.locked_until` for an hour with compare-and-set (so only one email goes), records `locked`, and emails the owner with the time it lifts. A locked account can't sign in by any method or step up. `Auth::unlock` lifts it. Wrong _emailed_ codes don't count toward a lock: anyone who knows an address can get those wrong, so counting them would let anyone lock anyone out. Tests: `tests/mfa.rs`, `repeated_wrong_second_factors_lock_the_account_and_email_it`.

### Replay

| Threat                                         | Control                                                                                                                               | Tests                                                               |
| ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| Reusing a TOTP code (shoulder-surfed, phished) | `totp.last_used_step`: a code's 30-second step is claimed with compare-and-set, so it works once, even across two concurrent requests | `signing_in_asks_for_the_second_factor_and_refuses_a_replayed_code` |
| Reusing a recovery code                        | `used_at` claimed with compare-and-set                                                                                                | `a_recovery_code_signs_in_once_and_ends_the_other_sessions`         |
| Reusing the second step                        | Single use, 5 minutes, retired after its last attempt or by a newer step for the same account                                         | `tests/mfa.rs`, `tests/http.rs`                                     |
| Reusing a refresh token                        | Rotation on every use; a second use after a 30-second grace ends the session and records `refresh_reused`                             | `src/sessions.rs` unit tests, `src/client/tests.rs`                 |

### Phishing

TOTP and emailed codes are **not** phishing-resistant: a site that relays them in real time gets a session. The controls narrow the window (codes work once, steps expire in 5 minutes) and the audit trail records the sign-in with its IP address. Passkeys are the answer on the web: the browser signs the origin into the client data, and `webauthn-rs` refuses any origin but `[auth] origin` and any RP ID but its host. Native passkeys arrive in Auth 6. NIST SP 800-63B rates TOTP as AAL2 but not phishing-resistant, and so do we; a user-verified passkey is AAL2 and phishing-resistant.

### Passkeys

| Threat                                      | Control                                                                                                                                                                                              | Tests                                                              |
| ------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| A signature made for another site           | `webauthn-rs` checks the client data's origin and the RP ID hash                                                                                                                                     | `passkeys_from_other_sites_cloned_keys_and_strangers_are_refused`  |
| A replayed assertion or registration        | The challenge lives server-side, sealed in `challenges`, and is claimed (compare-and-set) _before_ the answer is checked, outside the transaction that checks it, so it works once even when refused | `a_passkey_is_added…`, `passkeys_step_up_and_are_managed_after_it` |
| A cloned key                                | A signature counter that doesn't move forward is refused (`webauthn-rs`), and recorded as `passkey_failed`                                                                                           | `passkeys_from_other_sites…`                                       |
| Someone else's passkey, or a forged user ID | Discoverable sign-in looks the credential up by ID and requires its account to equal the user handle; the second step and step-up offer and accept only the account's own credentials                | `passkeys_from_other_sites…`, `an_emailed_code_then_asks…`         |
| A key that never verified its user          | User verification is `required` for registration and every assertion, and checked again on the result                                                                                                | by `webauthn-rs`, and `check_passkey`                              |
| A passkey added to persist after a takeover | Adding needs a fresh sign-in or step-up, rotates the session token, and emails the owner; removing needs step-up and ends other sessions; `reset_mfa` removes them all                               | `tests/passkeys.rs`                                                |
| Ceremony starts filling the table           | 120 starts per IP per 15 minutes; rows expire in 5 minutes                                                                                                                                           | limits                                                             |
| Login CSRF with a passkey ceremony          | The sign-in ceremony is found by the SHA-256 of a token in an `HttpOnly` `__Host-passkey` cookie, cleared once used                                                                                  | `a_browser_signs_in_with_a_passkey_through_its_cookie`             |

`webauthn-rs` uses OpenSSL, which the `passkey` feature builds from source (`openssl/vendored`), so servers don't depend on system headers. Attestation is `none`: we trust no authenticator vendor more than another. AAGUID-based names (the plan's bundled list) aren't built; passkeys are named after the device that added them.

### Third-party sign-in

| Threat                                              | Control                                                                                                                                                                                                                                                                 | Tests                                                                                                |
| --------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| Account takeover through an email claim ("nOAuth")  | Identity is `(provider, subject)`. A new subject never signs in to an existing account; a vouched-for address another account has gives `email_in_use` and creates nothing. Microsoft's email is never vouched for                                                      | `an_address_another_account_has_never_signs_in_to_it`, `google_github_microsoft_and_apple…`          |
| Forged ID tokens                                    | Signature with `ring` against the provider's JWKS (asymmetric algorithms only; the key's `kty`, `crv`, and `alg` must match; `crit` refused), then `iss` (tenant-aware for Microsoft), `aud`, `azp`, `exp`, `iat`, `nbf` with 60 s skew, and the nonce in constant time | `forged_replayed_and_misdirected_answers_are_refused`, `oauth.rs` unit tests (RFC 7515 RS256 vector) |
| Claims whose types skip a check                     | Times are `i64`: a string `exp` fails to parse rather than reading as absent                                                                                                                                                                                            | `claims_are_checked_and_times_must_be_numbers`                                                       |
| Code interception                                   | PKCE `S256` on every provider; the verifier never leaves the server                                                                                                                                                                                                     | the fake provider checks it                                                                          |
| Login CSRF / a callback finished in another browser | `state` finds the flow only together with the browser's `__Host-provider` binding cookie (the flow's subject is a hash of it); each `state` works once                                                                                                                  | `forged_replayed…`                                                                                   |
| Open redirects                                      | `redirect_to` must be a path on this site: starts with `/`, not `//` or `/\`, no backslashes or control characters                                                                                                                                                      | `only_paths_on_this_site_are_returned_to`                                                            |
| Cross-site POST to the callback (Apple)             | The layer lets callbacks through its same-site check; the handler only acts on a same-site POST. A cross-site POST gets a page that re-posts the fields from this site (escaped; CSP allows only its own hashed script)                                                 | `google_github_microsoft_and_apple_each_work_their_own_way`                                          |
| Key-set fetching as an amplifier                    | JWKS cached 24 h, refetched for an unknown `kid` at most once a minute; responses capped at 1 MiB; 10 s timeout, no redirects                                                                                                                                           | `an_openid_connect_provider_signs_people_up_then_in`                                                 |
| A discovery document pointing elsewhere             | Its `issuer` must equal the configured one, and every endpoint must be `https` (or `http` on localhost)                                                                                                                                                                 | fake provider                                                                                        |
| Leaked provider secrets                             | Only environment variables; `Provider`'s `Debug` omits them; Apple's client secret is minted per request and lives five minutes                                                                                                                                         | `provider_definitions_are_checked`                                                                   |

### Linking, unlinking, and merging

- **Linking** needs a fresh sign-in or step-up when it starts, and the session must still be live when the provider sends the person back. The owner is emailed.
- **Unlinking** needs the same, and the account must keep a way to sign in (a verified email, a passkey, or another identity). Tests: `the_last_way_to_sign_in_stays_and_merges_respect_two_step`.
- **Merging** needs, at the moment of merging: a fresh sign-in or step-up on the survivor; proof of the absorbed account (completing its provider sign-in, which created the offer); and, when the absorbed account has second factors, one of them (TOTP, recovery code, or passkey), counted toward _its_ lockout. The offer is bound to the survivor's session, lasts 10 minutes, works once, and needs `confirm: true`. The merge runs in one transaction; the tombstone in `merged_accounts` keeps old IDs resolving. Both sides' addresses (now all on the survivor) are emailed. Tests: `connecting_disconnecting_and_merging_accounts`, `the_last_way_to_sign_in_stays…`.

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

Someone who signs in through the inbox and turns two-step on (or adds a passkey) within 10 minutes can lock the owner out. The email tells the owner, and `Auth::reset_mfa` is the operator's answer: it removes the authenticator, recovery codes, and passkeys, lifts any lock, ends every session, records `mfa_reset`, and emails the account. There's no self-service reset: an attacker with the inbox would use it too. Tests: `an_administrator_resets_two_step_verification`.

**Required two-step.** With `mfa = "required"` or a matching `required_for` role, an `aal1` session is resolved as _enrolling_: `CurrentSession.enrolling` holds it, `signed_in` is `None`, the policy-scoped `Db` sees no one, `SignedIn` extractors and guards answer `mfa_required`, and only `/auth/session`, `/auth/mfa`, TOTP and passkey enrolment, step-up, and sign-out accept it. Roles are read per request for `aal1` sessions only, and never inside a transaction. The last factor of such an account can't be removed. Tests: `a_required_second_factor_limits_sessions_to_setting_one_up`, `a_session_that_must_set_up_two_step_reaches_only_that`.

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

The generated scaffold allows `http://localhost:3000` and `http://10.0.2.2:3000` for development. A release build that still lists them sends tokens over plain HTTP to whatever answers on the device's loopback. `lk auth doctor --release` fails on them, on a plain `http` `[auth] origin`, and on the development mailbox. **Gap:** it's a check to run (in CI), not something `leptoskit build --release` enforces.

### Keys

- One environment variable, several keys, newest first. The newest hashes and encrypts; any listed key verifies and decrypts, so rotation never breaks codes in flight or stored secrets.
- Hashing and encryption use different keys derived from each configured key (`HMAC(key, "leptoskit-auth encryption v1")` for AES), so no key is used for two algorithms.
- A sealed secret names its key ID. Removing a key that still encrypts a TOTP secret makes that account's second factor fail with a server error (`a stored secret was encrypted with key …`), naming the key and never the value. Recovery codes hashed under a removed key stop matching.
- `lk auth rotate-keys` (`Auth::rotate_keys`) re-encrypts TOTP secrets and live ceremony state under the newest key, each row with compare-and-set, so it's safe while the app runs and idempotent. Hashes (recovery codes, emailed codes) can't be re-keyed; the command says so. Tests: `rotating_keys_re_encrypts_secrets_under_the_newest`.

### Logs and errors

`Debug` for `SessionToken`, `Tokens`, `RefreshTokens`, `EmailCode`, `SecondFactor`, `SecondFactorCode`, `FactorCode`, `TotpSetup`, `RecoveryCodes`, `Keys`, the TOTP secret, `PasskeyOptions`, `PasskeyStep`, `PasskeyAssertion`, `AddPasskey`, `MergeAccounts`, `MergeInput`, `Provider`, `ProviderRequest`, `CallbackParams`, and `QrCode` prints no secret. `lk auth doctor` says whether each variable is set, never its value. Provider failures keep their detail in `AuthError::Provider` for the server; clients hear only `provider`. HTTP errors never carry database, mailer, or configuration details. Notification emails are best effort: a mailer failure doesn't undo the change, which the audit trail already holds.

## Assurance levels

| Session got there by                          | `aal`  | `mfa_at`        |
| --------------------------------------------- | ------ | --------------- |
| An emailed code, no second factor on          | `aal1` | none            |
| An emailed code, then TOTP or a recovery code | `aal2` | the second step |
| Confirming an authenticator                   | `aal2` | then            |
| Step-up                                       | `aal2` | then            |
| A user-verified passkey, on its own           | `aal2` | then            |
| Adding a passkey                              | `aal2` | then            |
| A third-party provider                        | `aal1` | none            |

An emailed code never counts as a second factor, since email is the first. Step-up looks at `mfa_at`, not only `aal`, so an `aal2` session from yesterday still has to step up.

## What's tested where

- `src/totp.rs`: RFC 4226 and RFC 6238 test vectors, replay and skew, RFC 4648 base32 vectors, `otpauth://` encoding.
- `src/secrets.rs`: sealing round trips, wrong context, tampering, truncation, rotation, a dropped key.
- `src/mfa.rs`: recovery code format, step-up windows.
- `tests/mfa.rs` (SQLite and Postgres): enrolment, re-authentication, the second step, attempts, replay, recovery codes, step-up, turning off, rate limits, audit events, notifications, nothing stored in the clear.
- `tests/http.rs`: the routes, cookies, the second step's cookie lifecycle, bearer flows, error codes.
- `src/client/tests.rs`: the same two-step scenario through the browser's cookie jar and the native token keeper.
- `crates/leptoskit_cli/tests/db.rs` (ignored, end to end): the scaffold signs in over HTTP, and its native guest loads in the interpreter.
- `tests/passkeys.rs` (SQLite and Postgres): registration, discoverable sign-in, the second step, step-up, management, phishing origins, cloned counters, strangers, disabled accounts, required two-step. A software authenticator (`tests/support/passkey.rs`) makes real P-256 attestations and assertions at test time.
- `tests/providers.rs` (SQLite and Postgres for the main flows): every provider flavor against a fake identity provider served from memory (`tests/support/oidc.rs`: discovery, JWKS, PKCE-checking token endpoint, ES256 ID tokens with keys made at test time), forged and replayed answers, `email_in_use`, linking, unlinking, merging with and without the absorbed account's second factor.
- `src/oauth.rs`: JWS parsing and verification (RFC 7515 RS256 vector, EdDSA, curve checks, `none`/HMAC refused), claim checks, Microsoft tenants.
- `src/client/tests.rs`: passkeys through the browser client with the software authenticator; native apps answering `unsupported` without asking the server.
- `crates/leptoskit_cli/src/auth_command.rs`: `doctor` (secrets never printed, release refusals), `provider add`, `rotate-keys`.

## Open items

- Native passkeys and system-browser provider sign-in (Auth 6); the client functions answer `unsupported` there today.
- Merge in "bring its data" mode, and the `lk db check` guard (Auth 4).
- AAGUID names for passkeys.
- `leptoskit build --release` enforcing what `lk auth doctor --release` checks.
- Property tests and fuzzing for cookie, token, code, and JWS parsing, per the plan's risks section.
- An external review before the first stable release.
