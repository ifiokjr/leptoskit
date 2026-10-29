# Plan: `leptoskit_auth` and `leptoskit_idp`

Status: **approved** (revision 2). Implementation is in progress; each phase lands as its own pull request.

Revision 2 records these decisions:

- **A3:** accounts are never linked automatically. Users link, unlink and merge accounts themselves.
- **A10:** Solana is in, framed as linking a wallet with proof of signing control first and signing in second.
- **A5:** apps that sign in through yours get pairwise subject identifiers by default; your own first-party clients can opt out.

This plan builds on [database-plan.md](database-plan.md). It depends on that plan's schema packs (§7), access policies and globals (§4.9), and `Public*` types.

## The problem

Every app needs sign-in, and every app gets it slightly wrong. Serverpod ships an auth module; Gel shipped `ext::auth` (OAuth providers, email, magic links, passkeys) and tied it into access policies through a `current_user` global. leptoskit should do the same, with a stricter security baseline and with the frontend left headless so the design stays yours.

There are two products here:

1. **`leptoskit_auth`**, for signing users in to your app:
   - passkeys, with fully secure WebAuthn;
   - third-party login: 20 providers, plus any OpenID Connect provider;
   - Solana wallets linked to a profile with proof of signing control, and optionally used to sign in;
   - self-service linking, unlinking and merging of accounts;
   - email codes, and passwords (opt-in);
   - MFA: TOTP, security keys, recovery codes and step-up;
   - sessions, account linking, recovery and audit.

   It is built in, and comes with a backend, headless hooks, and generated pages you restyle.
2. **`leptoskit_idp`**, for letting other apps sign in _with_ your app ("Sign in with MyApp"). It is an OAuth 2.1 and OpenID Connect provider: the job oxide-auth does, done better.

## What I found in your previous work

openbudget (at `850ff8b`) used `webauthn-rs` 0.5 with `danger-allow-state-serialisation`:

- The server kept the `PasskeyRegistration` state in a `json_cache` table, and the browser only kept the challenge options in local storage. That's the right split, and it carries over here as `auth.challenges`, with a TTL and encryption at rest.
- It also used `argon2` and AES-GCM-SIV token encryption with key indexes. Both carry over.

`webauthn-rs`, the kanidm project's crate, remains the recommended passkey library. For signing in through third parties, `openidconnect` and `oauth2` (ramosbugs) are the maintained choices.

oxide-auth:

- has a single maintainer;
- doesn't mention OpenID Connect in its README;
- leaves storage, consent UI and client management to you.

`leptoskit_idp` is built on our own database layer instead, so it includes all three.

## Security baseline

These are the standards we design and test against. Each control below cites what it satisfies.

- **OWASP ASVS 5.0, Level 2** for everything; Level 3 controls where they're cheap, such as phishing-resistant MFA.
- **NIST SP 800-63B (revision 4)** for authenticators, passwords, session lifetimes and assurance levels (AAL1 and AAL2).
- **RFC 9700 (OAuth 2.0 Security Best Current Practice):**
  - PKCE everywhere;
  - no implicit or password grants;
  - exact redirect URI matching;
  - refresh token rotation.
- **RFC 8252 (OAuth for native apps):** use the system browser, never an embedded webview, with claimed `https` redirects.
- **WebAuthn Level 3:** discoverable credentials, conditional mediation, user verification required for passwordless sign-in.
- **The OpenID Foundation conformance suite** for `leptoskit_idp`, run in CI.

## Architecture

```text
                   ┌───────────────────────── leptoskit_auth ─────────────────────────┐
browser / native ─▶│ HTTP routes (/auth/*)   server fns (#[api])   headless client hooks │
guest app          │        │                      │                         ▲          │
                   │        ▼                      ▼                         │          │
                   │   methods: passkey · oauth/oidc · email code · password · solana  │
                   │        │        mfa: totp · webauthn · recovery codes · step-up    │
                   │        ▼                                                           │
                   │   sessions (cookie on web, bearer in secure store on native)       │
                   │        │                                                           │
                   │        ▼                                                           │
                   │   auth.* tables (schema pack) ── policies ── cx.user_id() global ──┼─▶ your tables' policies
                   └────────────────────────────────────────────────────────────────────┘
                   ┌───────────────────────── leptoskit_idp ──────────────────────────┐
third-party apps ─▶│ /.well-known/openid-configuration  /oauth/{authorize,token,…}     │
                   │ consent (headless) · clients · grants · signing keys · scopes      │──▶ #[api(scope = …)]
                   └────────────────────────────────────────────────────────────────────┘
```

### Crates

| Crate                       | Contents                                                                                                                                                                                                                          |
| --------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `leptoskit_auth`            | Features: `server` (routes, server functions, the pack, providers), `client` (headless hooks; wasm-safe), and per-method features `passkey`, `oauth`, `email`, `password`, `totp`, `solana` (wallet linking and optional sign-in) |
| `leptoskit_idp`             | The OAuth 2.1 and OIDC provider. It depends on `leptoskit_auth` for the signed-in user                                                                                                                                            |
| `leptoskit_plugin_passkeys` | **New native plugin:** Apple `ASAuthorization` (passkeys and Sign in with Apple), Android Credential Manager, and the Windows/Linux fallback through the system browser                                                           |
| `leptoskit_plugin_web_auth` | **New native plugin:** system-browser auth sessions (`ASWebAuthenticationSession`, Android Custom Tabs, desktop loopback redirect)                                                                                                |
| Existing plugins reused     | `secure_store` (native session tokens), `linking` (claimed redirects), `local_authentication` (biometric unlock of a stored session on the device)                                                                                |

### The `auth` schema pack

These tables use the portable subset, so they run on Postgres and SQLite. They show up in your own migrations, under the `auth` schema (or the `auth_` prefix on SQLite).

| Table             | Purpose                                                                                                                                           | Sensitive data and protection                       |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------- |
| `accounts`        | One per person: `id`, `created_at`, `disabled_at`, `locked_until`                                                                                 | none                                                |
| `emails`          | Many per account: address (normalized), `verified_at`, `is_primary`. A verified address is unique                                                 | `private`                                           |
| `identities`      | Third-party logins: `(provider, subject)` unique, plus `email`, `email_verified` and a profile snapshot                                           | `private`                                           |
| `passkeys`        | Credential ID, `webauthn-rs` `Passkey` (public key and counter), user-given name, AAGUID label, backup flags, `last_used_at`                      | `private`; public keys only                         |
| `passwords`       | Argon2id hash (only with the `password` feature)                                                                                                  | `secret`                                            |
| `totp`            | Encrypted secret, `confirmed_at`, `last_used_step` (prevents replay)                                                                              | `secret`; AES-256-GCM-SIV with a key ID             |
| `recovery_codes`  | HMAC-SHA-256 of each code using a server pepper, and `used_at`                                                                                    | `secret`                                            |
| `sessions`        | SHA-256 of the token, account, `aal`, `authenticated_at`, `mfa_at`, idle and absolute expiry, device label, `revoked_at`                          | `secret` token hash                                 |
| `wallets`         | Linked wallets: `(chain, address)` unique, `label`, `can_sign_in`, `is_public`, `verified_at`, `last_proved_at`, `proof` (message or transaction) | `private` unless the user marks one public          |
| `challenges`      | Short-lived state: OAuth `state`, PKCE verifier and nonce; WebAuthn ceremony state; email code hash; SIWS nonce                                   | encrypted; TTL; single use                          |
| `merged_accounts` | Tombstones `(absorbed_id, survivor_id, merged_at)`, so old `Id<User>` references (URLs, caches) can redirect                                      | none                                                |
| `provider_tokens` | Optional: a provider's access and refresh tokens, for apps that call Google or GitHub APIs on the user's behalf                                   | `secret`; encrypted; off unless the app asks for it |
| `events`          | Audit trail: sign-in success and failure, MFA enrolled or removed, new device, linking, recovery. IP and user agent                               | `private`; retention set in config                  |
| `rate_limits`     | Fixed-window counters per (action, account) and (action, IP)                                                                                      | none; the `RateLimiter` trait allows other stores   |

### How the app's `User` table connects

Your `User` shares its primary key with `auth.accounts`. This is class-table inheritance (database-plan §4.6), so `Id<User>` and `Id<auth::Account>` refer to the same UUID:

```rust
#[db::table(name = "users", public, account)]   // `account`: users.id REFERENCES auth.accounts(id) ON DELETE CASCADE
pub struct User {
	pub id: Id<User>,
	pub handle: Handle,
	pub display_name: String,
	pub avatar_url: Option<Url>,
	#[db(flatten)]
	pub times: db::Timestamps,
}

/// Called once, inside the sign-up transaction, for every sign-up method.
impl auth::OnSignUp for User {
	async fn on_sign_up(tx: &db::Tx, account: Id<auth::Account>, profile: &auth::Profile) -> auth::Result<NewUser> {
		Ok(NewUser {
			id: account.into(),
			handle: Handle::suggest(tx, profile.preferred_username()).await?,
			display_name: profile.name().unwrap_or("New user").into(),
			avatar_url: profile.picture().cloned(),
		})
	}
}
```

If `on_sign_up` returns an error, the whole sign-up rolls back: no orphan accounts.

## Sessions

| Platform                | Transport                                                               | Notes                                                                                                                                                  |
| ----------------------- | ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Web                     | `__Host-session` cookie: `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/` | Server functions are POST. State-changing requests also check `Origin` and `Sec-Fetch-Site`.                                                           |
| Native guest            | `Authorization: Bearer` with a token held in `secure_store`             | Optionally unlocked by biometrics through `local_authentication`. The token never enters the wasm guest's linear memory longer than one request needs. |
| `leptoskit_idp` clients | OAuth tokens (see below)                                                | Separate from first-party sessions                                                                                                                     |

How the tokens work:

- **Opaque and server-side:** tokens are 256 random bits. The database stores only their SHA-256 hash, so a database leak exposes no usable session.
- **No JWTs for first-party sessions:** the session row makes revocation immediate. (decision A2)
- **Rotation:** a new token on sign-in, on MFA completion, on privilege change and on recovery. The old token is revoked, which prevents session fixation.
- **Expiry:** idle 14 days and absolute 90 days by default (configurable). Each session records its assurance level: `aal1` for a single factor, and `aal2` for MFA or a user-verified passkey.
- **Device management:** users can list sessions (device label, location from IP if you enable it, last seen) and revoke one or all. Changing a password or removing an MFA factor revokes all other sessions.

## Sign-in methods

### Passkeys (primary)

- **Library:** `webauthn-rs`.
  - **Relying party:** the RP ID is the registrable domain from `[auth] origin`; the allowed origins are the web origin plus the native app associations.
  - **Credentials:** user verification is `required` for passwordless sign-in, credentials are discoverable (usernameless), and attestation is `none`.
  - **Checks:** sign counters and backup flags are verified and stored.
- **Conditional UI (autofill):** the sign-in page's username field offers saved passkeys straight away. The headless hook starts a conditional-mediation ceremony on mount and aborts it if the user picks another method.
- **Multiple passkeys** per account, each named by its AAGUID ("iCloud Keychain", "1Password", "YubiKey 5"). The AAGUID list is bundled at build time, with no runtime fetch.
- **Native:** `leptoskit_plugin_passkeys` calls the platform APIs. `leptoskit::server` serves `/.well-known/apple-app-site-association` (`webcredentials`) and `/.well-known/assetlinks.json` from `[native]` config, so the associations can't drift from the app bundle IDs.
- **Ceremony state** lives in `auth.challenges`: encrypted, single use, expiring after 5 minutes, and bound to a random ID in an `HttpOnly` cookie or a native request header. It is never kept in browser storage.

### Third-party login (OAuth 2 and OpenID Connect)

The client flow:

- PKCE `S256`, `state`, and `nonce` for OIDC.
- `form_post` where the provider needs it (Apple).
- ID token checks: signature against a cached JWKS (refreshed when an unknown `kid` appears), `iss`, `aud`, `exp` and `nonce`.
- Redirect targets after sign-in are checked against an allow-list, which prevents open redirects.

**Identity is `(provider, subject)`, never email.** Several real account takeovers came from apps trusting a provider's `email` claim. The best known is "nOAuth": the email claim in Microsoft Entra ID is mutable and unverified. So:

1. Signing in with a `(provider, subject)` that's already known signs in to that account.
2. A new `(provider, subject)` always creates a new account. If its email matches an existing account, the user is told, and is offered a way to connect the two ([Linking and merging](#linking-unlinking-and-merging-accounts)). Accounts are **never linked automatically** (decision A3).
3. A provider's email is stored as verified only if the provider asserts verification (`email_verified`, GitHub's `/user/emails` `verified` flag, and so on). Even then, it's never used to link accounts automatically.

**Providers** (decision A6). Each provider is a data definition (endpoints, scopes, claim mapping, quirks) plus a docs page with setup steps, plus a fixture test. None needs bespoke flow code unless noted.

| #  | Provider                    | Protocol                                       | Email                                    | Setup quirks we handle                                                                                                                                                                         |
| -- | --------------------------- | ---------------------------------------------- | ---------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1  | Google                      | OIDC                                           | `email_verified` claim                   | none                                                                                                                                                                                           |
| 2  | Apple                       | OIDC-style ID token                            | verified; may be a private relay address | The client secret is an ES256 JWT that we mint and rotate from the `.p8` key. Uses `form_post`. The name arrives only on first sign-in, so we store it then. Native Sign in with Apple on iOS. |
| 3  | Microsoft (Entra, personal) | OIDC                                           | **never trusted for linking**            | The issuer varies by tenant (`common`, `consumers`, `organizations`, or a tenant ID); the `iss` check is tenant-aware                                                                          |
| 4  | GitHub                      | OAuth 2 + REST                                 | `/user/emails` with a `verified` flag    | The primary email may be private; we request `user:email`                                                                                                                                      |
| 5  | Facebook                    | OAuth 2 + Graph                                | when granted                             | Pinned Graph API version, set in config                                                                                                                                                        |
| 6  | X                           | OAuth 2 + PKCE + API                           | treated as unavailable                   | Requires PKCE even for confidential clients                                                                                                                                                    |
| 7  | Discord                     | OAuth 2 + REST                                 | `verified` field                         | none                                                                                                                                                                                           |
| 8  | LinkedIn                    | OIDC                                           | `email_verified`                         | Uses "Sign In with LinkedIn using OpenID Connect", not the legacy API                                                                                                                          |
| 9  | GitLab (incl. self-hosted)  | OIDC                                           | claim                                    | Base URL is configurable                                                                                                                                                                       |
| 10 | Slack                       | OIDC                                           | claim                                    | Workspace-scoped                                                                                                                                                                               |
| 11 | Twitch                      | OIDC                                           | claim (with `claims` request)            | Needs an explicit `claims` parameter                                                                                                                                                           |
| 12 | Spotify                     | OAuth 2 + REST                                 | unverified                               | none                                                                                                                                                                                           |
| 13 | Reddit                      | OAuth 2 + REST                                 | not available                            | Needs a distinctive `User-Agent` for the app; the app name only, never user data                                                                                                               |
| 14 | Amazon                      | OAuth 2 + profile API                          | provided                                 | none                                                                                                                                                                                           |
| 15 | Dropbox                     | OIDC                                           | `email_verified`                         | none                                                                                                                                                                                           |
| 16 | Bitbucket                   | OAuth 2 + REST                                 | `/user/emails` with a confirmed flag     | none                                                                                                                                                                                           |
| 17 | Notion                      | OAuth 2                                        | owner email when the owner is a person   | Grants are per workspace; we map the bot owner to an identity                                                                                                                                  |
| 18 | Zoom                        | OAuth 2 + REST                                 | provided                                 | none                                                                                                                                                                                           |
| 19 | TikTok                      | OAuth 2 (Login Kit)                            | not available                            | Uses `client_key` instead of `client_id`                                                                                                                                                       |
| 20 | Yahoo                       | OIDC                                           | claim                                    | none                                                                                                                                                                                           |
| +  | Any OIDC issuer             | OIDC discovery                                 | per issuer                               | Okta, Auth0, Keycloak, Authentik, Zitadel, a company's Entra tenant, or another leptoskit app running `leptoskit_idp`                                                                          |
| +  | Solana wallets              | Sign-In With Solana (wallet-standard `signIn`) | none                                     | Linking comes first, and signing in is optional for each wallet. See [Solana wallets](#solana-wallets-link-first-sign-in-optionally).                                                          |

Provider details shift over time (Apple's secret rotation, X's API tiers, Facebook's Graph versions). Each provider's docs page records the date it was last verified, and `lk auth doctor` flags definitions older than a set age.

**On App Store builds,** if you enable any third-party provider for iOS, `lk auth doctor` warns that Apple's App Review guideline 4.8 expects an equivalent privacy-focused option, and suggests enabling Sign in with Apple.

### Linking, unlinking and merging accounts

**The situation this solves.** Someone signs up with GitHub. Months later they tap "Continue with Google" with the same email, and now they have two accounts. We never merge these silently, because that's how account takeovers happen. Instead, the user can connect the two themselves, from either account, and the result is one account.

#### Linking a new sign-in method

The user opens Settings → Connected accounts and chooses "Connect Google". Once the Google sign-in completes, there are three possible outcomes:

| What we find                              | What happens                                |
| ----------------------------------------- | ------------------------------------------- |
| The Google identity is new                | It's attached to the current account. Done. |
| It already belongs to this account        | Nothing changes.                            |
| It already belongs to **another account** | The merge offer below.                      |

The same applies to passkeys (adding one to the current account) and Solana wallets (below).

At sign-in, if a new identity's email matches an existing account, the user sees: "An account with this email already exists. Sign in to it to connect Google, or continue with a separate account." Choosing "sign in to it" leads straight into the linking flow above.

#### The merge offer

```text
┌──────────────────────────────────────────────────────────────────────┐
│ This Google login already has its own account                        │
│                                                                      │
│ @ifiok-g · created 3 March 2026 · 12 posts · 1 passkey               │
│                                                                      │
│ Merge it into this account (@ifiok)?                                 │
│  ● Bring its data here, then delete it   (if the app supports it)    │
│  ○ Move only the login here, and delete its data                     │
│                                                                      │
│ This can't be undone.                     [Cancel]  [Merge accounts] │
└──────────────────────────────────────────────────────────────────────┘
```

The page is headless, like every auth screen: `use_merge()` provides the preview, the available modes and the confirm action, and `lk generate auth` writes a starting design.

**The merge is only offered if both of these hold:**

- The current account, the **survivor**, has signed in recently (step-up within 10 minutes).
- The user has just proved control of the other account, the **absorbed** one, as strongly as signing in to it would require. Completing its Google sign-in is the proof. If the absorbed account has MFA, its second factor is required too, so a merge can't bypass the other account's protection.

**What a merge does, in one transaction:**

1. **Sign-in methods move to the survivor:** identities, passkeys, wallets and verified emails, de-duplicated. The absorbed account's TOTP secret and recovery codes are discarded; the survivor's own MFA settings stay.
2. **App data:**
   - In "Move only the login" mode, the absorbed account's rows are deleted through the normal `on_delete` cascade.
   - In "Bring its data" mode, its rows are reassigned to the survivor, following each table's merge strategy (below).
3. **Clean-up:** the absorbed account's sessions are revoked. Its grants to third-party apps through `leptoskit_idp` are also revoked; their pairwise subject IDs belonged to the absorbed account, so those apps have to ask again.
4. **Tombstone:** the absorbed `accounts` row is deleted, and a `merged_accounts` row is written. Old profile URLs, cached IDs and live subscriptions resolve to the survivor through `auth::resolve_merged(id)`.
5. **Records:** audit events are written on both accounts, and a notification goes to every verified email of both.

**There is no un-merge.** The confirmation says so, and the merge needs an explicit confirm action, not just a click through.

#### Merge strategies come from the schema

The database layer already knows every foreign key that points at `users`, so a typical app needs no merge code. Each table can say how its rows move:

```rust
#[db::table(name = "posts", public, on_account_merge = reassign)]          // the default: UPDATE author_id
pub struct Post { … }

#[db::table(name = "user_settings", on_account_merge = keep_survivor)]     // 1:1 table: drop the absorbed row
pub struct UserSettings { … }

#[db::table(name = "follows", on_account_merge = reassign_skip_conflicts)]  // unique (follower, followee): drop duplicates
pub struct Follow { … }

#[db::table(name = "wallet_balances", on_account_merge = custom)]           // add the balances together
pub struct WalletBalance { … }

impl auth::OnMerge for User {
	/// Summary for the merge preview: "12 posts".
	async fn summary(db: &db::Tx, absorbed: Id<User>) -> auth::Result<auth::MergeSummary> { … }
	/// The survivor's `User` row is kept; copy anything worth keeping from the absorbed one.
	async fn merge_profile(tx: &db::Tx, survivor: &mut User, absorbed: &User) -> auth::Result<()> { … }
	/// Runs for every table marked `custom`.
	async fn merge_custom(tx: &db::Tx, table: db::TableName, survivor: Id<User>, absorbed: Id<User>) -> auth::Result<()> { … }
}
```

| Strategy                  | Effect                                                                        |
| ------------------------- | ----------------------------------------------------------------------------- |
| `reassign` (default)      | Moves the rows. A unique conflict aborts the whole merge with a clear message |
| `reassign_skip_conflicts` | Moves the rows, and drops the absorbed rows that would conflict               |
| `keep_survivor`           | Drops the absorbed account's rows                                             |
| `keep_newest`             | For a 1:1 table, keeps whichever row was updated more recently                |
| `custom`                  | Calls `OnMerge::merge_custom` in the same transaction                         |

A table that references `Id<User>` inside a unique constraint, and has no strategy, would fail a real merge. **`lk db check` refuses to pass** until such tables declare one, so that failure is found in CI, not by a user. "Bring its data" mode is only offered once every table is covered.

#### Unlinking

Any sign-in method can be removed, after step-up, as long as the account keeps:

- at least one sign-in method;
- a second factor, if MFA is required for this user.

Removing a third-party identity also deletes its stored `provider_tokens`.

### Solana wallets: link first, sign in optionally

The main use is **"link a Solana account to this profile"**: proving that this person, or this entity, has signing access to an address. Signing in with the wallet is an extra that the user switches on per wallet.

**Linking**, from Settings → Wallets → "Link a wallet":

1. **The server issues a challenge.** It's a Sign-In With Solana message containing:
   - the domain and URI;
   - the statement "Link this wallet to @ifiok on My App";
   - the account ID;
   - a nonce, `issued-at`, and an expiry 5 minutes later.

   The challenge is stored in `auth.challenges`.
2. **The wallet signs it** through the wallet-standard `signIn` feature, or `signMessage` for wallets that only support that.
3. **The server checks it:**
   - the ed25519 signature, with `ed25519-dalek`;
   - that the message is byte-for-byte the one issued;
   - the domain, nonce (single use), expiry and address.
4. **A `wallets` row is created** with `verified_at` and the signed proof kept for audit. If the address already belongs to another account, the [merge offer](#the-merge-offer) applies. A wallet belongs to exactly one account.

**Using linked wallets:**

```rust
// Server: which wallets has this user proved they control?
let wallets = auth::Wallet::owned_by(cx.user_id()).all(&db).await?;

// Before a payout or an identity-bound on-chain action: a proof from the last 2 minutes.
auth::wallets::require_proof(&wallet, minutes(2)).await?;   // Err(ProofRequired); the client hook prompts a re-sign

// Policies can use ownership directly.
#[db::policy(update, |v, cx| v.treasury.in_list(cx.wallets()))]
```

Client hooks: `use_link_wallet()`, `use_wallets()`, `use_wallet_proof()` and `use_solana_sign_in()`.

**Wallets are private by default.** `is_public` lets someone show a wallet on their profile, as `PublicWallet { address, label }`. Linking a wallet should never quietly announce it.

**Signing in with Solana** is allowed only for wallets where `can_sign_in` is on. The user opts in for each wallet; it's off when a wallet is first linked. A wallet signature counts as one factor (`aal1`), since a wallet key isn't a user-verified authenticator, so the MFA policy still applies.

**What message signing can't prove.** Off-chain signing proves control of a **keypair**. It can't prove control of:

- program-derived addresses, or multisigs such as Squads, which have no private key to sign with;
- addresses behind some hardware wallets with limited message-signing support.

For those, a later tier adds **on-chain proof**:

1. The user submits a transaction from the address (or its multisig) containing a memo with the challenge.
2. The server checks over RPC (`[auth.solana] rpc-url-env`) that the transaction is finalized, authorized by the right signer, and carries the exact memo.

The same `wallets(chain, address)` model could take other chains later, such as Ethereum through Sign-In with Ethereum (EIP-4361), but that isn't in this plan.

### Email codes and magic links

- **Codes are the default.** Links are optional, because corporate email scanners pre-fetch single-use links and consume them. When links are enabled, they open a page that asks for one click (a POST) before consuming the token.
- Codes are 8 characters, valid for 10 minutes and single use. They're stored hashed and limited to 5 attempts, after which the code is invalidated.
- **No account enumeration.** "If that address can sign in, we've sent a code" is shown in every case, and the response timing is the same.
- **Email delivery** goes through a `Mailer` trait. SMTP (`lettre`) and a webhook mailer are built in (the webhook posts a signed JSON event to a URL you control). No vendor SDKs. (decision A7)

### Passwords (opt-in)

Passwords are off by default: passkeys and email codes cover the same ground with less risk. (decision A1) When enabled:

- **Hashing:** Argon2id with OWASP's parameters, re-hashed on sign-in whenever the parameters change.
- **Rules, following NIST 800-63B:**
  - at least 8 characters, and up to 64 or more;
  - no composition rules;
  - a local blocklist of the most common passwords;
  - an optional breach check against Have I Been Pwned using k-anonymity. It's off by default because it's an outbound request; the docs say what is sent: 5 hex characters of a SHA-1 hash, never the password.
- **Reset** uses the email code flow and revokes all sessions.

## Multi-factor authentication

| Factor                  | Counts as                                                                | Notes                                                                                                                                           |
| ----------------------- | ------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| Passkey (user-verified) | `aal2` on its own, because it's phishing-resistant and verifies the user | The recommended default                                                                                                                         |
| Security key            | second factor                                                            | WebAuthn without the discoverable requirement                                                                                                   |
| TOTP                    | second factor                                                            | RFC 6238, SHA-1, 30 s, 6 digits for app compatibility; ±1 step; replay is blocked through `last_used_step`. Enrollment needs a confirming code. |
| Recovery codes          | second factor, once each                                                 | 10 codes, shown once, regenerating invalidates all of them. Using one revokes other sessions and sends a notification.                          |
| Email code              | **not** a second factor when email is also the first factor              | Never lets a single factor count twice                                                                                                          |

**Policy:**

- `[auth] mfa = "optional" | "required"`, plus per-role requirements (`required-for = ["Admin"]`).
- **Step-up** for sensitive actions, usable from server functions and policies:

```rust
#[api]
#[server]
pub async fn delete_account() -> Result<(), ServerFnError> {
	let session = auth::require(auth::Aal::Two.within(minutes(10)))?;   // Err(AuthError::StepUpRequired) otherwise
	…
}
```

The client hook `use_step_up()` catches `StepUpRequired`, runs whichever factor the user has (a passkey prompt, or a TOTP field), and retries the call.

## Abuse protection

| Threat                           | Control                                                                                                                                                                                       |
| -------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Credential stuffing, brute force | Rate limits per account and per IP on every attempt endpoint; exponential backoff; temporary lock (`locked_until`) that notifies the account by email                                         |
| Enumeration                      | Identical responses and timing for "no such user" and "wrong code"                                                                                                                            |
| CSRF                             | `SameSite=Lax`, POST-only server functions, an `Origin`/`Sec-Fetch-Site` check on state-changing requests, and a `state` parameter on OAuth                                                   |
| Session fixation                 | Rotation at every privilege change                                                                                                                                                            |
| Token theft from the database    | Only hashes of session, code and recovery tokens are stored; TOTP secrets and provider tokens are encrypted with a key outside the database                                                   |
| Open redirects                   | Allow-listed `redirect_to` values (exact match, or a same-origin path)                                                                                                                        |
| Account takeover via email       | Identity is `(provider, subject)`; linking needs explicit proof of both sides                                                                                                                 |
| Merge abuse                      | A merge needs recent step-up on the survivor and full sign-in proof of the absorbed account, including its MFA. Both accounts' emails are notified. There's no merge API without both proofs. |
| Wallet proof replay              | Single-use nonces bound to the domain and account, a 5-minute expiry, and a byte-exact message comparison                                                                                     |
| Phishing                         | Passkeys first. MFA prompts name the origin, and passkeys are bound to the RP ID.                                                                                                             |
| Timing attacks                   | Constant-time comparisons (`subtle`) for codes and token hashes                                                                                                                               |
| Logging leaks                    | `secret` fields are redacted in `Debug` and `tracing`; tokens are never logged; audit events store IP and user agent, never credentials                                                       |
| Lost devices                     | Recovery codes; session revocation from any device; a "new sign-in" email notification (configurable)                                                                                         |
| Key rotation                     | `AUTH_ENCRYPTION_KEYS` holds several keys with IDs: the newest key encrypts, and any listed key decrypts. `lk auth rotate-keys` re-encrypts in the background.                                |

All secrets come from environment variables, which Monosecret can resolve. `leptoskit.toml` only ever names environment variables, and the config schema rejects inline secrets.

## The developer experience

### Configuration

```toml
[auth]
app-name = "My App"
origin = "https://myapp.com"
methods = ["passkey", "email-code", "oauth", "solana"] # "password" is opt-in
mfa = "optional"
required-for = ["Admin"]
session = { idle = "14d", absolute = "90d" }
encryption-keys-env = "AUTH_ENCRYPTION_KEYS"

[auth.solana]
sign-in = true # allow wallets the user has opted in to sign in with
rpc-url-env = "SOLANA_RPC_URL" # only needed for on-chain proofs (phase Auth 7)

[auth.mail]
from = "My App <hello@myapp.com>"
smtp-url-env = "SMTP_URL"

[auth.providers.google]
client-id-env = "GOOGLE_CLIENT_ID"
client-secret-env = "GOOGLE_CLIENT_SECRET"

[auth.providers.apple]
client-id-env = "APPLE_SERVICES_ID"
team-id-env = "APPLE_TEAM_ID"
key-id-env = "APPLE_KEY_ID"
private-key-env = "APPLE_PRIVATE_KEY" # the .p8 contents

[auth.providers.github]
client-id-env = "GITHUB_CLIENT_ID"
client-secret-env = "GITHUB_CLIENT_SECRET"
```

### Setup instructions, from the CLI

```console
$ lk auth provider add google
Google sign-in: 4 steps (about 5 minutes)

 1. Open https://console.cloud.google.com/apis/credentials and create an
    "OAuth client ID" of type "Web application".
 2. Add these Authorized redirect URIs:
      http://localhost:3000/auth/callback/google       (dev)
      https://myapp.com/auth/callback/google           (prod, from [auth] origin)
 3. Set the environment variables (for example through Monosecret):
      GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET
 4. Run `lk auth doctor` to check the configuration.

Added [auth.providers.google] to leptoskit.toml.
```

`lk auth doctor` checks:

- that each environment variable is present; it never prints the values;
- that redirect URIs are `https` in production;
- that the RP ID matches the origin, and that the well-known association files match the native bundle IDs;
- that Apple key expiry is far enough away;
- the age of each provider definition;
- the App Store guideline 4.8 warning above.

Each provider also has a docs page, `docs/src/guide/auth/providers/<name>.md`, with screenshots.

### Server wiring

```rust
// src/main.rs
let db = Db::connect_from_env().await?;
db.migrate(my_app_db::MIGRATIONS).await?;
let auth = auth::Auth::from_config::<User>(&config, db.clone())?;   // User: OnSignUp

let app = Router::new()
	.leptos_routes_with_context(&options, routes, auth.context(), App)   // provides Db (policy-scoped) and Session
	.merge(auth.router())                                               // /auth/* callbacks, WebAuthn, email links
	.merge(my_app_backend::router())
	.layer(auth.layer());                                               // resolves the session and sets cx.user_id()
```

### Server functions and policies

```rust
#[api]
#[server]
pub async fn my_drafts() -> Result<Vec<PublicPost>, ServerFnError> {
	auth::require_signed_in()?;
	let db = Db::from_context()?;            // policies see cx.user_id() from the session
	let drafts = Post::query().filter(|p| p.published_at.is_null()).all(&db).await?;
	Ok(drafts.into_iter().map(Into::into).collect())
}
```

The `posts` policy `author_id = cx.user_id()` (database-plan §4.9) already limits this query to the caller's drafts. The filter here is only about which drafts to show; it isn't a security check.

### The headless client

```rust
use leptoskit::auth::client::*;

#[component]
pub fn SignIn() -> impl IntoView {
	let auth = use_auth();                            // Signal<AuthState>: Anonymous | SignedIn(PublicSession) | NeedsMfa(MfaChallenge)
	let passkey = use_passkey_sign_in();              // starts conditional UI on mount
	let email = use_email_code();                     // .send(addr), .verify(code), .state()
	let providers = use_providers();                  // enabled providers: id, display name, brand metadata
	let wallet = use_solana_sign_in();

	view! {
		<form on:submit=move |e| { e.prevent_default(); email.send(email_input.get()); }>
			<input type="email" autocomplete="username webauthn" />   // passkey autofill
			…
		</form>
		<For each=move || providers.get() key=|p| p.id.clone() let:provider>
			<button on:click=move |_| provider.sign_in()>{provider.display_name.clone()}</button>
		</For>
	}
}
```

**`lk generate auth`** writes complete, Tailwind-styled pages into your app, which you own:

- sign-in, sign-up, email verification, MFA challenge;
- security settings: passkeys, TOTP, recovery codes, sessions, connected accounts;
- consent, if `leptoskit_idp` is enabled.

This is the shadcn approach: the hooks are the API, and the pages are a starting point you restyle freely. Regenerating never overwrites a file you've edited unless you pass `--force`.

### Development

- **Dev provider:** `[auth.providers.dev]` adds a "Sign in as…" page that lists seeded users. It exists only in debug builds; release builds get a `compile_error!` if it's configured.
- **Dev mailbox:** in `lk dev`, outgoing email goes to `/_leptoskit/mail` instead of SMTP, so you can sign in with email codes without an inbox.
- **Tests:**
  - `auth::testing::sign_in_as(&user)` builds a session for server-function tests.
  - The WebAuthn tests use `webauthn-rs`'s software authenticator.

## `leptoskit_idp`: Sign in with your app

Your app becomes an OpenID Connect provider. Other apps, including other leptoskit apps, CLIs and TVs, can sign users in through it and call your `#[api]` functions on their behalf.

### Better than oxide-auth

| oxide-auth                           | `leptoskit_idp`                                                                                                      |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------- |
| OAuth 2 primitives; storage is yours | Storage is the `idp` schema pack; clients, grants, codes, refresh families and keys are tables with policies         |
| No OIDC mentioned                    | OIDC Core: discovery, JWKS, ID tokens, `userinfo`, RP-initiated logout                                               |
| Consent UI is yours to build         | Headless consent hooks plus a generated page; users manage grants under "Connected apps"                             |
| Framework adapters                   | Native to leptoskit: same sessions, same policies, same config                                                       |
| Grants you assemble yourself         | OAuth 2.1 defaults: code + PKCE only, rotating refresh tokens, exact redirects                                       |
| Scopes are strings                   | Typed scopes, enforced on `#[api]` and published in the OpenAPI spec, so generated clients know what each call needs |
| Single maintainer, limited time      | Maintained here and tested against the OpenID conformance suite                                                      |

### Protocol surface

| Endpoint                                              | Standard                      | Tier |
| ----------------------------------------------------- | ----------------------------- | ---- |
| `/.well-known/openid-configuration`, `/oauth/jwks`    | OIDC Discovery, RFC 7517      | 1    |
| `/oauth/authorize` (code + PKCE `S256` required)      | RFC 6749, RFC 7636, OAuth 2.1 | 1    |
| `/oauth/token`: `authorization_code`, `refresh_token` | RFC 6749                      | 1    |
| `/oauth/userinfo`                                     | OIDC Core                     | 1    |
| `/oauth/revoke`                                       | RFC 7009                      | 1    |
| `/oauth/introspect`                                   | RFC 7662                      | 1    |
| `/oauth/end_session`                                  | OIDC RP-Initiated Logout      | 1    |
| `/oauth/par`                                          | RFC 9126 (pushed requests)    | 2    |
| `/oauth/device_authorization` + device grant          | RFC 8628 (CLIs and TVs)       | 2    |
| `client_credentials` grant                            | RFC 6749 (machine to machine) | 2    |
| DPoP sender-constrained tokens                        | RFC 9449                      | 3    |
| Dynamic client registration                           | RFC 7591                      | 3    |

**Deliberately absent:** the implicit grant, the password grant, the hybrid flow, `plain` PKCE, and wildcard redirect URIs.

**Tokens and keys** (decision A4):

- **ID tokens** are ES256 JWTs. Signing keys rotate on a schedule, and retired keys stay in JWKS until every token signed with them has expired.
- **Access tokens** are 10-minute JWTs following the RFC 9068 profile (`aud` is your API), so resource servers verify them offline. Introspection covers immediate revocation.
- **Refresh tokens** are opaque, stored hashed, and rotate on every use. If a used refresh token is ever presented again, the whole token family is revoked (reuse detection).
- **Client authentication:** `private_key_jwt` (preferred), `client_secret_basic` (with the secret stored hashed), or `none` for public clients, where PKCE is the protection.

**Privacy** (decision A5): third-party clients get **pairwise subject identifiers** by default. Each client sees a different, stable `sub` for the same user, so two apps can't correlate users through your IdP. Your own first-party clients can opt in to the public `sub`.

### Typed scopes on your RPC

```rust
#[derive(idp::Scopes)]
pub enum Scope {
	/// Read your posts and drafts.
	#[scope("posts:read")]
	PostsRead,
	/// Create, edit and delete your posts.
	#[scope("posts:write")]
	PostsWrite,
}

#[api(tag = "posts", scope = Scope::PostsRead)]
#[server]
pub async fn list_my_posts() -> Result<Vec<PublicPost>, ServerFnError> { … }
```

How these scopes are used:

- **On a call:** a third-party call carries a bearer token. `#[api(scope = …)]` checks the token's scopes, and `cx.user_id()` is the user who granted consent, so your policies apply unchanged.
- **On consent:** the consent screen lists the scopes using their doc comments.
- **In generated clients:** the OpenAPI spec gains an `oauth2` security scheme with these scopes, so `lk rpc generate` builds TypeScript, Dart, Python and Go clients that know which scope each call needs.

That makes "sign in with my app, then call my API" a single feature.

**Claims** are mapped from your types, and nothing is released unless the scope allows it:

```rust
impl idp::Claims for User {
	fn claims(&self, scopes: &idp::GrantedScopes, out: &mut idp::ClaimSet) {
		if scopes.has(idp::Profile) {
			out.name(&self.display_name).preferred_username(self.handle.as_str());
		}
		if scopes.has(idp::Email) {
			out.verified_email_from_account();   // only a verified primary email is ever released
		}
	}
}
```

### Managing clients

- **Command line:** `lk auth idp client add "Partner App" --redirect https://partner.example/callback --scopes openid,profile,posts:read`. It prints the client ID once, and the secret once.
- **Developer portal:** `lk generate auth --idp` adds headless developer-portal pages, so your users can register their own apps if you want third-party developers (owner-scoped clients, guarded by policies).
- **Users** see and revoke grants under "Connected apps", and revoking kills the refresh family.

### Conformance

A nightly CI job runs the OpenID Foundation conformance suite (Basic OP, Config OP, PKCE, and later FAPI-style checks) against a test server. Passing it is the release gate for `leptoskit_idp`. The same test server also serves as the mock "any OIDC issuer" for `leptoskit_auth`'s client tests, so neither side needs live third-party providers in CI.

## Delivery

These phases interleave with the [database phases](database-plan.md#delivery). Auth needs db phase 4 (policies, globals and packs). Each phase is one PR, or a `gh stack`.

| Phase  | Contents                                                                                                                                                                                                                                                  |
| ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Auth 1 | The `auth` pack; sessions on web and native; email codes; the `Mailer` trait; the dev provider and mailbox; rate limits; audit events; `OnSignUp`; `require_*` guards; the `cx.user_id()` global; headless `use_auth`; `lk generate auth` (sign-in pages) |
| Auth 2 | Passkeys on the web (conditional UI, management); TOTP; recovery codes; step-up; the security settings page; a threat model document in `docs/agents/`                                                                                                    |
| Auth 3 | The OIDC and OAuth client core, with Google, Apple, GitHub and Microsoft first, plus generic OIDC; linking and unlinking; account merge in "move only the login" mode; `lk auth provider add` and `doctor`; provider docs pages                           |
| Auth 4 | Solana wallets (linking, fresh proofs, opt-in sign-in, `cx.wallets()` in policies); merge in "bring its data" mode, with `on_account_merge` strategies and the `lk db check` guard; the merge preview                                                     |
| Auth 5 | The remaining 16 providers in batches, with fixtures and docs for each; optional `provider_tokens`; opt-in passwords                                                                                                                                      |
| Auth 6 | Native: `leptoskit_plugin_web_auth` (system-browser OAuth), `leptoskit_plugin_passkeys`, native Sign in with Apple, well-known association files                                                                                                          |
| Auth 7 | On-chain wallet proof for multisigs, PDAs and hardware wallets that can't sign messages                                                                                                                                                                   |
| IdP 1  | `leptoskit_idp` tier 1: discovery, JWKS, code + PKCE, tokens, `userinfo`, revoke, introspect, logout, consent, typed scopes on `#[api]`, connected apps, client CLI, conformance in CI                                                                    |
| IdP 2  | PAR, the device flow (including `lk login` for your own CLI), client credentials, the developer portal                                                                                                                                                    |
| IdP 3  | DPoP, dynamic registration, certification submission                                                                                                                                                                                                      |

**As built (Auth 1, first layer: the pack, sessions and email codes):**

- **Crate and features:** `leptoskit_auth` with `server`, plus `postgres` and `sqlite`, which pick the database. The facade's `auth` feature follows `db-server`, `db-postgres` and `db-sqlite` through weak dependency features.
- **Tables:** `accounts`, `emails` (named `EmailAddress`, since `Email` is the scalar), `sessions`, `challenges`, `events` and `rate_limits`. The other tables wait for the phases that use them.
  - Each table has a `select` policy that shows the signed-in person only their own rows.
  - `challenges` and `rate_limits` deny every read.
  - The crate's own queries run on `db.system("auth: …")`.
- **The app's `User` table:** no `account` table option was needed. The app keys its table by `id: Id<auth::Account>` with `#[db(on_delete = cascade)]`; a non-`Self` key is both the primary key and a foreign key. `SignedIn::user_id::<U>()` converts the account ID for policies.
- **`OnSignUp`:** it inserts the app's row itself rather than returning `NewUser`, because the crate has no generic way to insert another crate's row. It runs inside the sign-up transaction, as system.
- **`cx.user_id()`:** it is `AuthContext::signed_in(|me| …)` for conditions, and `user_id::<U>()` for values. Both are on `Context`, through a trait.
- **Email codes:**
  - Codes are stored as HMAC-SHA-256 under the newest key in `Keys`, which is `AUTH_ENCRYPTION_KEYS` as `id:base64url` pairs. The hash is bound to the purpose and the address, and any listed key verifies.
  - Attempts are counted with compare-and-set before the answer is checked. The fifth wrong guess retires the code.
  - Sending a new code retires older ones.
- **Rate limits:** fixed windows, held as `rate_limits` rows keyed by a SHA-256 of the action and source, and updated with compare-and-set (eight tries, then refused). This is behind the `RateLimiter` trait.
- **Admin controls:** `Auth::disable` and `enable` were added, so `disabled_at` means something before an admin UI exists.
- **Left for the next layers:**
  - the SMTP and webhook mailers;
  - `[auth]` config;
  - `use_auth` and `lk generate auth`.

**As built (Auth 1, second layer: HTTP):**

- **Features:** `axum` adds `Auth::layer()` and `Auth::router()`; `leptos` adds the server-function guards. The facade's `server` feature turns on `leptoskit_auth?/leptos`.
- **The layer:**
  - It is a hand-written tower `Layer` and `Service` (`SessionLayer`, `SessionService`), not `from_fn`, so `Auth::layer()` returns a nameable type.
  - It runs the same-site check first, then resolves the session from `Authorization: Bearer` (preferred) or the cookie.
  - It inserts `CurrentSession` and a policy-scoped `Db` as request extensions. The app's handlers read `Extension<Db>` from the layer instead of adding their own, which is how "no route builds a policy-scoped `Db` before the session is resolved" holds.
- **Same-site check:**
  - Safe methods pass.
  - Otherwise a present `Origin` must equal the configured `origin`, or, without one, the `Host` header's authority.
  - Otherwise `Sec-Fetch-Site` must be `same-origin` or `none`.
  - Otherwise the request passes only without the session cookie, which is how native apps and scripts call.
  - This also stops login CSRF on `/auth/email/verify`.
- **Cookie:** `__Host-session` with `Secure`, unless `origin` is `http://…`, which gives a plain `session` cookie for local development. `Max-Age` is the absolute lifetime.
- **Routes:** JSON in and out. The wire types (`SendCode`, `VerifyCode`, `Transport`, `SignedInResponse`, `SessionResponse`, `RevokeSession`, `ErrorResponse`) build without any feature, for the client hooks.
- **Errors:** `AuthError` implements `IntoResponse` with stable codes and `Retry-After`. Database, config, mailer and sign-up-hook details never reach the client.
- **Extractors:** `SignedIn` (`401` when signed out), `Option<SignedIn>` through `OptionalFromRequestParts`, and `RequestInfo`.
- **Client IP:** the `ConnectInfo<SocketAddr>` peer by default, or the first address in a trusted proxy header named with `client_ip_header`.
- **Guards:** `require_signed_in()`, `signed_in()`, `request_db()` and `current_session()` read the request `Parts` that leptos_axum provides as context. They are synchronous; there's no `Db::from_context`.
- **Development tools:** `dev_mailbox()` and `dev_sign_in()` exist only under `cfg(debug_assertions)`, so configuring them in a release build fails to compile. The routes are only added in debug builds, and `Auth::dev_sign_in` re-checks at runtime. The "Sign in as…" list of seeded users became a single route that takes any address, since a JSON route needs no page.

## Decisions

| #   | Decision                         | Recommendation                                                                                                                         | Trade-off                                                                                                                                                   |
| --- | -------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A1  | Passwords                        | Off by default; opt-in feature                                                                                                         | Some audiences expect passwords; passkeys and email codes remove a whole class of breaches                                                                  |
| A2  | First-party sessions             | Opaque, database-backed tokens                                                                                                         | One read per request (cacheable), in exchange for instant revocation and no JWT pitfalls                                                                    |
| A3  | Account linking                  | **Decided:** never automatic. Users link and unlink methods themselves, and can merge two accounts irreversibly with proof of both     | One extra step for users, in exchange for closing the most common OAuth takeover. Merging is permanent, so it needs the strongest proof we ask for anywhere |
| A11 | Merge modes                      | "Move only the login" always available; "bring its data" once every table referencing `Id<User>` has a merge strategy                  | Reassigning data is what users expect, but only the schema can make it safe, which is why `lk db check` enforces coverage                                   |
| A4  | IdP access tokens                | Short-lived JWTs plus introspection                                                                                                    | Offline verification for resource servers, at the cost of up to 10 minutes' lag on revocation without introspection                                         |
| A5  | Pairwise `sub` for third parties | **Decided:** on by default                                                                                                             | Matches your privacy stance; first-party clients can opt out                                                                                                |
| A6  | Provider coverage                | 20 named providers plus generic OIDC plus SIWS, delivered as data, docs and fixtures                                                   | Ongoing upkeep as providers change, which is why definitions are dated and checked by `doctor`                                                              |
| A7  | Email delivery                   | SMTP and webhook built in; the `Mailer` trait for anything else                                                                        | No vendor SDKs to maintain; apps that want Resend or SES write about 20 lines                                                                               |
| A8  | Headless UI delivery             | Hooks in `leptoskit_auth/client`, plus `lk generate auth` for pages you own                                                            | Generated code goes stale as the hooks evolve; the hooks are the stable API                                                                                 |
| A9  | MFA default                      | `optional`, with per-role `required-for` and per-call step-up                                                                          | Required MFA everywhere hurts sign-up conversion; step-up protects what matters                                                                             |
| A10 | Solana in the auth module        | **Decided:** yes, behind the `solana` feature. Linking wallets with proof of signing control comes first; sign-in is opt-in per wallet | Treating a wallet as a proven attribute of a profile fits how it's used (payouts, ownership); sign-in is a convenience on top                               |

## Risks

- **Security bugs here are the expensive kind.** Mitigations:
  - a written threat model (phase Auth 2);
  - property tests and fuzzing for token, cookie and SIWS message parsing;
  - the OIDC conformance suite;
  - an external review before the first release that's marked stable.
- **Provider drift.** The 20 providers will change behaviour under us. Dated definitions, `doctor` warnings and fixture tests keep this visible, and the generic OIDC path covers anyone we lag on.
- **Native passkeys and system-browser auth** need two new plugins on five platforms, and Windows and Linux have weaker platform APIs. Those two fall back to the system browser, and the docs say so.
- **The `cx.user_id()` coupling.** Policies trust whatever the auth layer sets. That boundary needs the most tests: no route may build a policy-scoped `Db` before the session is resolved.
- **Merges destroy data, and permanently.** Mitigations:
  - a test that every foreign key to `users` in the exported schema is covered by a strategy;
  - a test that the preview matches what the merge actually does;
  - fault injection mid-merge, to prove the transaction leaves nothing half-done;
  - a test that a merge can't run with a stale step-up or without the absorbed account's MFA.
- **Scope.** Treat phases Auth 1–3 plus IdP 1 as the first complete release, and everything after as increments.
