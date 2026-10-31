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
  - `lk generate auth`;
  - the `--auth` scaffold.

**As built (Auth 1, fourth layer: the headless client):**

- **Server functions, not `fetch`:** the client is four leptos server functions under `/auth/rpc/`: `session`, `send_code`, `verify_code` and `sign_out`. They are wrapped in leptoskit_query scopes (`session()`, `send_code()`, `verify_code()`, `sign_out_mutation()`) and `use_auth()`. That's how the rest of leptoskit reads the server (harbor, the fetching-data guide), and it works during SSR with no second code path. The plan's `use_passkey_sign_in`, `use_providers` and `use_solana_sign_in` arrive with their phases.
- **`ErrorResponse`:** it implements `FromServerFnError`, so the client keeps the same error codes as the JSON routes. Transport failures map to `network`, `invalid` or `server`, without details.
- **Server side:** the `ssr` feature reads the `Auth`, the `CurrentSession` and the `Db` that the layer now puts in request extensions, and sets or clears the cookie through leptos_axum's `ResponseOptions`.
  - Without a database feature the server functions compile, and answer `server` errors.
  - `auth.router()` doesn't mount `/auth/rpc` itself, because `leptos_routes` already mounts every registered server function, and axum panics on duplicate routes.
- **Facade:** the `auth` feature turns on `client`, and `server` turns on `ssr`.

**As built (Auth 1, fifth layer: native tokens and devices):**

- **Why:** native apps have no cookie jar, and a single 14-day bearer token can't be rotated or show that it was copied. Native apps now hold an access token and a refresh token; browsers keep the `HttpOnly` cookie, which scripts can't read and which is the safest store a browser has.
- **Opaque tokens, not JWTs:** both are 256 random bits, stored only as SHA-256 hashes on the `sessions` row and checked on every request. There's no signing key to steal, no algorithm to confuse, nothing for a script to read, and revocation is immediate. A JWT's only gain, offline verification, is worth nothing here: every request already reads the database for its policy-scoped `Db`.
- **The session row is the grant:**
  - `token_hash` is the cookie's token or the access token.
  - `access_expires_at` (default 15 minutes, `[auth] access_minutes`, at most 60) bounds the access token. It is `NULL` for cookies.
  - `refresh_hash`, `previous_refresh_hash` and `refreshed_at` hold the rotation state, so sign-out, revocation and the device list need no second table.
- **Refresh (`POST /auth/refresh`, `Auth::refresh`):**
  - Each refresh token works once.
  - The update compare-and-sets on the current `refresh_hash`, so a concurrent second refresh is refused rather than forking the session.
  - The previous token is accepted for 30 seconds, for a response lost on a mobile network. A retry inside the grace issues a fresh pair, keeps the presented token as the previous one so the grace doesn't stretch, and kills the pair that never arrived.
  - After the grace, a reused token revokes the session and records `refresh_reused`. That's OAuth 2.1 reuse detection, with the session as the token family.
- **`RequestInfo` gained `transport` and `device_name`:**
  - The layer sets `Bearer` when the request carried one, so `rotate` keeps a native session native.
  - `VerifyCode` carries an optional `device_name` (trimmed, at most 100 characters) for the device list.
- **Devices:**
  - `Auth::devices` lists live sessions as `Device { session, current }`, and `GET /auth/sessions` now returns those.
  - `Auth::sign_out_everywhere(account)` ends every session, the current one included. It backs `POST /auth/sessions/revoke-all`, and `disable` now shares it.
  - The client adds the server functions `sessions`, `revoke_session`, `revoke_other_sessions` and `sign_out_everywhere`, with scopes `devices()`/`use_devices()`, `revoke_device()`, `revoke_other_devices()` and `sign_out_everywhere_mutation()`.
- **`SignedInResponse.token` became `tokens: Option<Tokens>`.** `Tokens` and `RefreshTokens` print no secrets in `Debug`.
- **Left for the next layer:** the native client itself: a token keeper over `secure_store` and the HTTP plugin that refreshes one request at a time, since native guests have no server functions.

**As built (Auth 1, sixth layer: one client for every platform):**

- **Why:** the fourth layer's server functions only existed on the web, so a native app would have needed a second client with its own API. The owner's rule: the same functions, arguments and behaviour on every device, with no platform branches in app code.
- **One public API** in `leptoskit_auth::client`, unchanged in name: `session()`, `use_auth()`, `send_code()`, `verify_code()`, `sign_out_mutation()`, `devices()`, `use_devices()`, `revoke_device()`, `revoke_other_devices()`, `sign_out_everywhere_mutation()`.
  - `verify_code()` takes `EmailCode { email, code }`; the client picks the transport. Its `Debug` leaves out the code.
  - `revoke_other_devices()` and `sign_out_everywhere_mutation()` return `()`, because the JSON routes answer `204` without a count. `revoke_device()` still returns whether the session was live (`204`/`404`).
  - `configure(ClientConfig { origin, device_name })` is the one setup call, the same everywhere. Nothing else told a guest where its server is: `[auth] origin` and `[native.plugin-options.http] origins` live on the server and the host, and the guest can read neither. On the web the origin defaults to the page's own. The device name is sent only with bearer sign-ins, since the server names browsers from their user agent. Calling `configure` again installs a fresh client.
- **One implementation** (`client/protocol.rs`, `Remote<H, C>`) speaks the `/auth/*` JSON routes, over two internal seams:
  - `Http` sends a request: `fetch` in the browser (`client/browser.rs`), `leptoskit_plugin_http::guest::HttpClient` in a guest (`client/native.rs`), and an in-memory axum router in the tests. `HttpRequest::headers()` is shared, so the host tests check what every sender attaches.
  - `Credentials` attaches credentials, keeps what a sign-in returns, forgets them on sign-out, and decides after a refusal whether a retry is worth it.
  - `GET /auth/session` answers a stale bearer token with `{ session: null }`, not `401`, so for that route a null session counts as a refusal too. Without that, a native app whose access token lapsed would look signed out while its refresh token was still good.
- **Credential holders, picked by target and feature:**
  - `wasm32` + `web`: `Cookie`, which holds nothing. The fetch is `credentials: "same-origin"`, `mode: "same-origin"`. `leptoskit_plugin_http`'s browser backend is not used, because it sets `credentials: "omit"` and CORS mode. Nothing is kept in script-readable memory or storage.
  - `wasm32` + `guest`: `TokenKeeper`, with the access token in memory and the refresh token in `leptoskit_plugin_secure_store` under `leptoskit.auth`/`refresh_token`.
    - It renews when the access token is within `min(60 s, lifetime / 2)` of expiring, by the clock `leptoskit_query::now_ms`, and after a refusal.
    - Renewals are single-flight behind a `futures::lock::Mutex`: a waiter re-checks freshness (or, after a refusal, whether the token changed) before trading again. This matters because a second use of a refresh token after the 30 s grace ends the session.
    - A `signed_out` refusal of the refresh clears the store and memory. A network failure keeps the refresh token, since a retry within the grace still works.
    - Sign-out forgets the tokens even when the request fails.
    - A guest host installs no clock today, so `now_ms` reads `0` and renewal happens after the server's `401` (or null session) rather than a minute early. Installing a host clock turns on the early renewal with no client change.
  - `ssr` on a non-Wasm target: `client/server.rs`, which reads the session and devices in process through the guards, with no HTTP hop. Its mutations answer `server`, since they run in the browser. Without a database feature the reads answer `server` too.
  - Anything else (for example `client` alone): `Unreachable`, which answers `config` from every call.
- **New client error codes:** `config` (not set up to reach the server: no origin in a guest, a missing plugin, or a denied origin) and `storage` (the secure store failed). `network` stays. `invalid` went with the server functions.
- **Removed:** the `/auth/rpc/*` server functions, `client_server.rs`, `ErrorResponse`'s `FromServerFnError` impl, the `leptos_axum` dependency, and `tests/client.rs`. Nothing else used them: server rendering now reads in process, and a native guest can't contain server functions.
- **Features:** `client` (scopes and the shared implementation), `web` (browser fetch), `guest` (HTTP and secure-store plugins, as `wasm32`-only dependencies), and `ssr` (in-process reads). The facade's `web` turns on `leptoskit_auth?/web`, `guest` turns on `leptoskit_auth?/guest`, and `server` still turns on `leptoskit_auth?/ssr`.
- **Tests** (`client/tests.rs`) drive the real `Auth::router()` and `Auth::layer()` in memory:
  - The same scenario runs against the cookie holder, through a sender that keeps a cookie jar and sends `Origin` as a browser does, and against the token holder with an in-memory secure store: sign in, the session query, devices with `current`, revoke one, revoke others, sign out everywhere, sign out.
  - Native-only cases: renewing before expiry, renewing after the server refuses the token, concurrent requests sharing one renewal, a failed renewal clearing the store, a restart restoring the session from the stored refresh token, and an unreachable server keeping the refresh token.
  - The scopes run against a real `QueryClient`, and the SSR reads run inside the layer under a leptos owner.
- **Left for later:** a host-provided clock for guests; registering `http` and `secure-store` automatically in the Windows and Linux hosts; and a cookie-less browser mode for an app served from a different origin than its API, which this deliberately doesn't support.

**As built (Auth 1, seventh layer: generated pages and the scaffold):**

- **The screens** (`crates/leptoskit_cli/src/auth_pages/templates/`) are leptoskit `Element` trees over the headless client, not leptos views, so one file renders on the web and in a native guest. Each is a `Copy` state struct (`SignIn`, `Devices`) of signals and query handles with a `screen()` method, like the scaffold's `Counter`.
  - `auth::provide()` creates both in the current owner, which needs a `QueryClient`, and puts a `Screens` in context; `auth::screens()` reads it. Context rather than a global keeps one request's sign-in state from leaking into another's during server rendering.
  - `Screens::gate(|| home)` shows the sign-in screen until someone is signed in, then `home` under a bar with "Where you're signed in" and "Sign out".
  - `SignIn` reports nothing about the session until an `Effect` marks it mounted. Effects never run during server rendering, so the server always renders "Checking…", and the browser's first render matches it however quickly the in-process session read resolves. Without this, a loaded server sometimes rendered the signed-out form, and hydration would have met different markup.
  - Text inputs take their value untracked (as harbor's search does), so typing doesn't re-render the tree and drop focus. The email is checked loosely before sending; the server's check is the real one.
  - After `rate_limited`, a `spawn_local` task counts a signal down from `retry_after` with `leptoskit_query::sleep`, and the buttons stay disabled until it reaches zero. On a guest without a host timer, `sleep` resolves at once, so the countdown ends immediately and the server's limit still holds.
  - The devices query is enabled only while someone is signed in, so a signed-out screen never asks for it.
  - The screens' pure helpers (`looks_like_email`, `retry_after`, `sentence`, `day`) carry unit tests, which run in the generated app.
- **`lk generate auth`** (`auth_pages.rs`):
  - It finds the client crate from `build.client_package` (the root package, or a workspace member), writes `src/auth/`, declares `pub mod auth;`, and adds the `auth`, `leptos`, and `query` features to its `leptoskit` dependency with `toml_edit`, keeping the manifest's formatting.
  - `leptos` is the facade's re-export, so the screens can import signals in a native build, where the app's own `leptos` dependency is off.
  - With the file router, it adds `sign-in.rs` and `devices.rs` to the routes directory, unless a route file by that name exists anywhere under it, then regenerates the routes module. A failed regeneration becomes a next step, not an error.
  - Without `[auth]`, it appends one to `leptoskit.toml`, after checking the result still loads.
  - With a server (`src/main.rs` and `leptoskit`'s `server` feature) and a `[db]`, it writes `src/sign_in.rs` (`service::<U: OnSignUp>(&db)`: `Auth::from_config` over the embedded `leptoskit.toml`, plus `dev_mailbox()` and `dev_sign_in()` under `cfg(debug_assertions)`), declares it, and turns on `auth`. Without a database it says so and stops there.
  - It never edits the app's own code beyond appending module declarations. Router composition, `auth::provide()`, and the user table become numbered next steps, each printed only while its marker (`auth::provide(`, `sign_in::service`) is missing.
  - Idempotence: a file that matches is skipped silently, a differing one is reported as kept unless `--force`, and a run that changes nothing says so.
- **`leptoskit new --auth`** implies `--db sqlite` unless `--db` is given. It reuses the same page and server-module templates, plus:
  - `db_auth.rs`: `packs!(leptoskit_auth)`, and a `User` keyed by `Id<auth::Account>` whose `OnSignUp` names the user from the email's local part.
  - `server_auth.rs`: it merges `auth.router()` and adds `auth.layer()` last.
  - `backend_auth.rs`: the backend no longer adds its own `Extension(db)`, so handlers get the layer's policy-scoped `Db`.
  - `app_auth.rs`: `provide()` in the web `App`, and `configure(ClientConfig::new())` in `hydrate`/`start`. In the native entry it calls `configure` with a `SERVER` origin constant, installs an `any_spawner` local executor over a `futures` `LocalPool`, and exports `leptoskit_poll`. The pool runs after every event and plugin reply, and then renders again.
  - The scaffold leaves `[auth] origin` unset, so a browser can use either `localhost` or `127.0.0.1`, and the cookie is `__Host-session`. The README says to set it before deploying.
- **Tests:**
  - Unit tests cover generation on temporary projects: first run, a second run changing nothing, kept and forced files, the router with and without a clashing route, a failed regeneration, a single-crate app, missing crates, and every dependency shape.
  - Snapshots pin the `--auth` scaffold, `generate auth`'s files and output on a `--db` scaffold, a second run's output, and the new help text.
  - The ignored end-to-end test in `tests/db.rs` builds `new --auth` against this repository. It migrates, builds the server, runs the generated tests, and checks the app for `wasm32` with `hydrate` and `native`. It then signs in with a code read from `/_leptoskit/mail`, reads the session with the cookie, and posts a note under the auth layer.
- **Left for later** (the first three were fixed in the Auth 2 first layer, below):
  - The scaffold doesn't grant `http` and `secure-store` in `[native] plugins`, because the native registry requires each to be a direct dependency of the host runner, and those dependencies aren't wired or verified yet.
  - Separately, the facade and `leptoskit_auth` take `leptoskit_query` with its default `browser` feature. The scaffold's `native` guest build imports wasm-bindgen's `setTimeout`, `Date.now`, `Window`, and `navigator` glue from it, outside the interpreter's shim set, so a host can't instantiate it until guest builds turn that feature off.
  - Both block running the screens in a native host. They compile for `wasm32` with `native` today; the server is verified end to end, and the browser bundle builds.
  - Sign-up and email verification pages wait for their phases. (MFA's came in the Auth 2 first layer, and `--idp`'s with the IdP stack.)

**As built (Auth 1, third layer: mailers and config):**

- **`[auth]` in `leptoskit.toml`:** `app_name`, `origin`, `keys_env`, `client_ip_header`, `idle_days` and `absolute_days`, plus `[auth.mail]` with `from`, `smtp_url_env`, `webhook_url_env` and `webhook_secret_env`.
  - Keys are snake_case, like the rest of the file.
  - Lifetimes are whole days rather than the plan's duration strings, so there's no parser to get wrong.
  - Only `_env` fields can carry a secret, and `deny_unknown_fields` rejects anything else, which is how "the config schema rejects inline secrets" holds.
  - Validation: `https` origins, or `http` on localhost only; idle ≤ absolute; at most one mailer, and whole ones.
- **`LeptoskitConfig::from_toml`:** new, so a server embeds its configuration with `include_str!` and needs no file at runtime. `load_from` now calls it.
- **`Auth::from_config(&db, &config)`:** returns the builder, so the app still adds `on_sign_up` (and `dev_sign_in`). A configured mailer whose feature is off is an error, not a silent fallback. With no mailer, debug builds use the dev mailbox and release builds refuse to start.
- **Mailers:**
  - `SmtpMailer` uses lettre with rustls and the webpki roots, behind the `smtp` feature.
  - `WebhookMailer` uses reqwest, behind the `webhook` feature. It signs per Standard Webhooks (`webhook-id`, `webhook-timestamp`, and `webhook-signature: v1,<base64 HMAC>` over `id.timestamp.body`, with `whsec_` secrets decoded); the spec's own test vector is a unit test.
  - Neither error message ever includes the SMTP URL or the webhook URL, which may hold credentials.
- **Facade:** the features are `auth-smtp` and `auth-webhook`.

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

**As built (Auth 2, first layer: native guests, TOTP, recovery codes, and step-up):**

- **Native guests load (the Auth 1 follow-ups):**
  - `leptoskit_query`'s workspace dependency is `default-features = false`, so nothing gets `browser` by accident. The facade's `web`, `csr`, and `hydrate` turn on `leptoskit_query?/browser` and `leptoskit_rpc?/browser`; `leptoskit_auth/web` turns on `leptoskit_query/browser`. `leptoskit_rpc` gained a default `browser` feature, so its direct users see no change.
  - **A guest clock.** The interpreter had no time import. Rather than add one, which a guest built against a newer SDK would need from every older host, the host now calls an optional `leptoskit_clock(unix_ms: i64)` export with the wall clock before every entry point (in `refuel`), like `leptoskit_viewport`. `leptoskit_query`'s new `guest` feature defines that export and reads it in `now_ms`; the facade's `guest` and `leptoskit_auth/guest` turn it on. Old hosts never call it and the clock stays at zero, as before; old guests don't export it and the host skips it. The query crate's `forbid(unsafe_code)` became `deny`, with one `allow` on the export. There's still no host timer: sleeps resolve at once, so retries don't back off and the sign-in countdown ends immediately.
  - **The shims.** Even without `browser`, a guest that uses leptos signals links js-sys's futures and stream glue, because `leptos` turns on `any_spawner/wasm-bindgen` and `server_fn/browser` unconditionally. A call-graph pass over the scaffold's release guest showed each such import is reachable only through function tables (trait-object vtables) or wasm-bindgen's own JS-facing exports, never from an entry point a host calls. So the interpreter now links every function import from wasm-bindgen's two modules that isn't one of its harmless shims to a trap that names it. Imports from any other module still fail instantiation. This replaces the old closed set's rule that unknown wasm-bindgen imports fail to load; the test for that now asserts the trap on call, and a new one keeps the load failure for other modules.
  - **Plugin grants.** `--auth` writes `plugins = ["http", "secure-store"]`, an `[native.plugin-options.http]` with `http://localhost:3000` and `http://10.0.2.2:3000`, and both plugin crates in the iOS, Android, and macOS host manifests. `lk generate auth` does the same for an existing app with those hosts: it adds the crates beside each host's `leptoskit_bridge_ffi`, at its version and from its checkout when it uses a path, and grants the plugins with `toml_edit`, keeping any `origins` the app already chose.
  - **Desktop runners.** The Rust Linux, Windows, and terminal runners host no plugins at all, and used to refuse to build when any were granted. `registry::resolve` now builds them without `native.plugins`, with a warning, so one `leptoskit.toml` builds everywhere; `rust_desktop::generate` keeps its refusal as a backstop. Sign-in there answers `config` until those runners can host plugins.
  - **Proof.** The ignored `the_auth_scaffold_signs_people_in` test now builds the `--auth` scaffold's native guest in release, checks its import section has none of the browser APIs our crates use (timers, `Date`, `navigator`, `Window`, `fetch`, listeners, `BroadcastChannel`) and only imports from `leptoskit`, `leptoskit_plugins_v1`, and wasm-bindgen's modules, checks it exports `leptoskit_clock`, then loads it in `WasmHost::headless`, starts it, and finds the sign-in screen's text. `crates/leptoskit_wasm/tests/clock.rs` covers the clock export.
- **Tables:** `totp` (`TotpSecret`: one per account, `secret` sealed, `confirmed_at`, `last_used_step`) and `recovery_codes` (`RecoveryCode`: HMAC, `used_at`). Both deny every read. `Purpose::SecondFactor` joins `challenges`, and `EventKind` gains `TotpEnabled`, `TotpDisabled`, `SecondFactorFailed`, `SteppedUp`, `RecoveryCodeUsed`, and `RecoveryCodesRegenerated`.
- **Encryption at rest:** `Keys::seal` and `Keys::open`, AES-256-GCM-SIV under a key derived from each configured key (`HMAC(key, "leptoskit-auth encryption v1")`), with a random 96-bit nonce, the purpose and account as associated data, and the key ID in a one-byte-length header (so key IDs are now at most 64 bytes). Any listed key opens; a missing key is a `Config` error naming the key.
- **TOTP** (`src/totp.rs`): RFC 6238 with HMAC-SHA-1, 30 s steps, 6 digits, ±1 step, written here rather than taken from a crate since it's 40 lines and the RFC vectors pin it. Secrets are 160 bits. Verification computes all three windows and compares in constant time, and the matched step must be after `last_used_step`, which is then claimed with compare-and-set.
- **Recovery codes:** 10 per account, 50 bits each in the emailed codes' alphabet (`XXXXX-XXXXX`), HMAC'd with the account in the input, all compared on each attempt. Made when an authenticator is first confirmed and the account has none, or by `regenerate_recovery_codes`.
- **Sign-in:** `verify_email_code` now returns `SignInStep`: `SignedIn(SignIn)`, or `SecondFactor(SecondFactorChallenge)` when the account has a confirmed authenticator. The challenge is a 256-bit token, stored as SHA-256 in `challenges` with the account as its subject, for 5 minutes and 5 attempts (counted before checking, like email codes). A newer one retires older ones. `complete_sign_in(token, code, request)` starts an `aal2` session with `authenticated_at` from the first step and `mfa_at` now. `dev_sign_in` skips the second step.
- **Step-up:** `Auth::step_up` checks a TOTP or recovery code, then moves the session to `aal2` with `mfa_at` now and **replaces its token in place** (new access and refresh tokens for a native session), so the device list keeps one row. `SignedIn::stepped_up_within` and `require_step_up`, the `require_step_up(window)` guard for server functions, and `STEP_UP_WINDOW` (10 minutes) are the API.
- **Management:** `begin_totp` (needs a sign-in within the window, or step-up once a factor exists; `AlreadyEnrolled` when one is on; restarting replaces an unconfirmed secret), `confirm_totp` (elevates and rotates like step-up), `disable_totp` and `regenerate_recovery_codes` (step-up; disabling deletes the codes and ends other sessions), and `mfa_status`.
- **Recovery code use** ends every other session. Turning an authenticator on or off and using a recovery code email the primary address, best effort: the audit trail already has the event.
- **Limits:** `Limits::second_factor_per_account` (10 per 15 minutes) for every TOTP and recovery attempt, plus the existing `verify_per_ip`.
- **Errors:** `SignInExpired` (`sign_in_expired`, 400), `StepUpRequired` (`step_up_required`, 403), `Reauthenticate` (`reauthenticate`, 403), `AlreadyEnrolled` and `NotEnrolled` (409). A wrong TOTP or recovery code is `invalid_code`.
- **Routes:** `POST /auth/second-factor`, `GET /auth/mfa`, `POST /auth/step-up`, `POST /auth/totp`, `POST /auth/totp/confirm`, `POST /auth/totp/disable`, `POST /auth/recovery-codes`. `POST /auth/email/verify` answers `VerifyResponse`, untagged: a `SignedInResponse` as before, or `{ second_factor: { factors, expires_in, token? } }`. A browser's second-step token goes in an `HttpOnly` `__Host-second-factor` cookie for its lifetime, never in the body; a native app's (`transport: "bearer"`) is in the body. The cookie is cleared once the step succeeds or ends, and kept after a wrong code. The same-site fallback now also refuses origin-less requests that carry that cookie.
- **Client, the same everywhere:** `verify_code()` returns `SignInStep` (`SignedIn(PublicSession)` or `SecondFactor(SecondFactorPrompt)`). New scopes: `complete_sign_in()`, `mfa()`/`use_mfa()`, `step_up()`, `enroll_totp()`, `confirm_totp()` (returns `TotpConfirmation`), `disable_totp()`, `regenerate_recovery_codes()`. `Credentials` gained `hold_second_factor`/`second_factor`: the cookie holder holds nothing; `TokenKeeper` keeps the step's token in memory only, and forgets it on sign-out or `sign_in_expired`. Rotated native tokens from step-up and confirmation are kept. Server rendering reads `mfa()` in process.
- **Screens:** `lk generate auth` writes `auth/security.rs` (`Security`: turn on with the key and link shown as text, confirm, show recovery codes once, make new ones, turn off; it answers `step_up_required` by asking for a code, stepping up, and retrying the change, and `reauthenticate` by offering to sign out) and `/security` for the file router. The sign-in screen gained the second step, and the account bar a "Two-step verification" tab. A new CLI test checks every class the templates use against the native Tailwind compiler; it caught `font-mono` and `text-left`, which would have trapped on a device.
- **Tests:** `tests/mfa.rs` (SQLite and Postgres), new cases in `tests/http.rs` and `src/client/tests.rs` (whose browser jar now keeps cookies by name, like a browser), unit tests in `totp.rs`, `secrets.rs`, `mfa.rs`, and `guards.rs`. Test-side TOTP is computed independently in `tests/support/totp.rs`. Tests age pack rows with raw `UPDATE`s through `Connection::execute`, since the pack's writers are private.
- **Left for the next layers of Auth 2:** all built in the second layer, below.

**As built (Auth 2, second layer: passkeys, required two-step, lockout, keys):**

- **Passkeys** (`src/passkeys.rs`, feature `passkey`): `webauthn-rs` 0.5 with `conditional-ui`, `danger-allow-state-serialisation` (state is sealed server-side), and `danger-credential-internals` (read only for the backup flag). Its OpenSSL is built from source (`openssl/vendored`), because CI builds `--all-features` on Windows and macOS, which have no OpenSSL headers. The relying party is `[auth] origin`'s host; without an origin, passkey calls answer a configuration error.
  - **Table:** `passkeys` (`Passkey`): credential ID (unique), the serialized `webauthn_rs::Passkey` (public key, counter), name (1–100 characters), `backed_up`, `last_used_at`. Select policy shows the owner their rows.
  - **Ceremonies:** a new nullable `challenges.state` column holds the sealed state, bound to purpose, subject, and the row's hash. New purposes `PasskeyRegistration` (subject: the session), `PasskeyAuthentication` (subject `sign-in`, `second-factor:{challenge}`, `step-up:{session}`, or `merge-passkey:{session}`), `Provider`, `ProviderSignUp`, and `Merge`. `take_ceremony` claims a row before anything is checked, outside the checking transaction, so each works once whatever the answer. Five minutes for passkeys.
  - **Registration** asks for a discoverable, user-verified passkey (it overrides `webauthn-rs`'s `require_resident_key(false)` in the options it sends) and excludes the account's own. It needs a fresh sign-in or step-up, rotates the session to `aal2` like confirming TOTP, issues recovery codes when the account has none, records `passkey_added`, and emails. A credential ID already registered answers `passkey_failed`.
  - **Sign-in** is discoverable (`start_discoverable_authentication`): the user handle must equal the stored credential's account. It starts an `aal2` session with `mfa_at` now. The browser holds the ceremony's token in `__Host-passkey`; a native app gets it in the body.
  - **The second step and step-up** share TOTP's path through a `Proof` (`Code` or `Passkey`) in `mfa.rs`, so attempts, failures, events, and lockout are the same. `Factor::Passkey` comes first in `factors()`.
  - **Management:** `passkeys`, `rename_passkey`, `remove_passkey` (step-up; ends other sessions; refuses the last factor of an account that must have one, or the last way to sign in). Recovery codes are deleted only when no factor is left for them to back up (so turning TOTP off keeps them while a passkey remains).
  - **Without the feature,** every ceremony method exists and answers `AuthError::Unsupported` (`unsupported`, 501).
- **Routes:** `POST /auth/passkey/{sign-in,second-factor,step-up,register}/options` and the four that finish them, `GET /auth/passkeys`, `POST /auth/passkeys/{rename,remove}`. The same-site fallback also refuses origin-less requests carrying `__Host-passkey` or `__Host-provider`.
- **Wire types:** `webauthn-rs-proto` is a non-optional dependency (serde types only), so `api.rs` carries `CreationChallengeResponse`, `RequestChallengeResponse`, `PublicKeyCredential`, and `RegisterPublicKeyCredential` for every client. `MfaStatus` gained `passkeys` and `required` (`#[serde(default)]`), and `enrolled()`.
- **Required two-step:** `[auth] mfa = "optional" | "required"` and `required_for = [...]` (snake_case, like the rest of the file), `MfaPolicy`, and the `Roles` trait (`AuthBuilder::roles::<U>()`; building with `required_for` and no roles hook is a configuration error). `resolve_session` marks an `aal1` session of such an account as _enrolling_; `resolve` treats it as signed out. `CurrentSession.enrolling` holds it; the policy `Db` sees no one; `SignedIn` extraction and the guards answer `MfaRequired` (`mfa_required`, 403); a private `Enrolling` extractor lets the session, MFA status, TOTP and passkey enrolment, and step-up routes accept it. Roles are read before any transaction, since SQLite's in-memory pool has one connection.
- **Lockout:** `Limits::failures_before_lock` (20 per day) and `lock_for` (one hour). Each wrong second factor (`InvalidCode` or `PasskeyFailed`) hits a per-account counter through the `RateLimiter`; the first refusal sets `locked_until` with compare-and-set, records `locked`, and emails. Emailed codes don't count. `Auth::unlock` records `unlocked`. Step-up now refuses a locked account too.
- **Admin reset:** `Auth::reset_mfa` deletes the authenticator, recovery codes, and passkeys, clears the lock, ends every session, records `mfa_reset`, and emails.
- **Keys:** `Auth::rotate_keys` (`src/rotate.rs`) reseals TOTP secrets and live ceremony state under the newest key with compare-and-set, returning `Rotated { totp, ceremonies, current }`. `Keys::newest()` and `Keys::ids()` are public.
- **QR code:** `client::qr_code(text)` (the `qrcode` crate, no default features, under `client`) returns `QrCode` with `side()`, `path()` (SVG path data for a `Vector`, which every platform but HarmonyOS draws), `is_dark`, and `svg(module)`. Its `Debug` hides the modules.
- **`TextInput::autocomplete`:** a new `NativeProps.autocomplete` (web renders it as `autocomplete`; native renderers ignore it), so the email field can say `username webauthn` and browsers offer passkeys in its suggestions. Unknown props are ignored by every renderer and by serde, so old hosts are unaffected.
- **Client** (the same everywhere): `passkey_sign_in()` with `PasskeyMode::{Prompt, Autofill}`, `cancel_passkey_sign_in()`, `add_passkey()`, `complete_sign_in_with_passkey()`, `step_up_with_passkey()`, `passkeys()`/`use_passkeys()`, `rename_passkey()`, `remove_passkey()`. A third seam, `Platform`, does what the device itself does: the browser's (`client/browser.rs`) calls `navigator.credentials` through `webauthn-rs-proto`'s `wasm` conversions, with one `AbortController` at a time, and checks `isConditionalMediationAvailable` for autofill; `NotAllowedError`/`AbortError` become `cancelled`. Native apps (bearer transport) answer `unsupported` before asking the server anything. Server rendering reads `passkeys()` in process.
- **Screens:** the sign-in screen gained a passkey button, autofill started on mount (failing quietly), the passkey as a second step, and provider buttons and outcomes (below). The account bar's tab is now "Security": passkeys (add, remove), the authenticator with its QR code, connected accounts, and the merge offer, with step-up by passkey or code. Checked by compiling the templates in a scratch crate on the host and for `wasm32` with `guest`.
- **Facade:** `auth-passkeys` and `auth-providers` turn on `leptoskit_auth/passkey` and `/oauth`, for the server's crate only. The `--auth` scaffold doesn't turn them on yet (the vendored OpenSSL build is slow), so its server answers `unsupported` until the app does.

**As built (Auth 3, first layer: providers, linking, merging, and the CLI):**

- **The client core** (`src/oauth.rs`, feature `oauth`) is written here rather than with `openidconnect`: it's the authorization code flow with PKCE `S256`, `state`, and `nonce`, plus ID token verification with `ring` (already in the graph through rustls). `jsonwebtoken`'s pure-Rust backend pulls `rsa` 0.9 (RUSTSEC-2023-0071, unfixed), which `cargo deny` refuses; its 9.x line had a claim type-confusion advisory. Accepted algorithms: RS256/384/512, PS256/384/512, ES256, ES384, EdDSA; the JWK's `kty`, `crv`, `alg`, and `use` must agree; `crit` is refused; times are `i64`, so a string `exp` fails to parse.
- **Providers** are data (`Provider::google`, `apple`, `microsoft`, `github`, `oidc`), each with endpoints and quirks, and a `VERIFIED` date (`PROVIDERS_VERIFIED`) that `doctor` ages: Google's issuer may also be `accounts.google.com`; Apple mints an ES256 client secret per request from the `.p8` (five minutes), uses `form_post`, sends the name once (`user` parameter), and may send `email_verified` as a string; Microsoft's issuer must be the token's own tenant (`tid`), within `common`/`organizations`/`consumers`/a tenant ID, and its email is never vouched for; GitHub reads `/user` and `/user/emails` (the primary address's `verified`). Generic OIDC uses discovery, whose `issuer` must match exactly and whose endpoints must be `https` (or `http` on localhost).
- **HTTP to providers** goes through the `ProviderHttp` trait: `reqwest` by default (10 s timeout, no redirects, 1 MiB cap), or `AuthBuilder::provider_http` in tests. JWKS and discovery are cached for a day; an unknown `kid` refetches at most once a minute.
- **The flow:** `POST /auth/oauth/start` (or `/link`, which needs a fresh session) answers `{ url }` and sets `__Host-provider` (ten minutes). The flow is sealed in `challenges` under a subject derived from that cookie, and found by the `state` token, so a callback only finishes in the starting browser. `GET`/`POST /auth/callback/{provider}` answers `303` to the flow's `redirect_to` (a same-site path) with `?auth=<outcome>` (`ProviderOutcome`: `signed_in`, `second_factor` with `auth_factors`, `linked`, `already_linked`, `merge`, `email_in_use`, or `error` with `auth_error`). The layer lets callback paths past its same-site check; a cross-site `POST` (Apple's `form_post`) gets an auto-posting page with a hashed-script CSP, and only the same-site re-post acts. Native (bearer) starts answer `unsupported`.
- **Identity:** `identities` (`Identity`): `(provider, subject)` unique, email as given with `email_verified`, name, `last_used_at`. A known identity signs in (or starts the second step, with its cookie); a new one creates an account, unless the provider vouched for an address another account has: then `email_in_use`, with the pending sign-up sealed under `ProviderSignUp`, and `POST /auth/oauth/continue` creates a separate account without that address. `on_sign_up` gets a `Profile` with the vouched-for email, name, username, and picture.
- **Linking and unlinking:** a new identity attaches (with its vouched-for address if no one has it), records `identity_linked`, and emails. `GET /auth/identities`, `POST /auth/identities/unlink` (fresh session; keeps a way to sign in; `identity_unlinked`).
- **Merging, "move only the login":** connecting an identity that belongs to another account stores a `Merge` offer under the session (ten minutes). `GET /auth/merge` describes it (`MergeOffer`); `POST /auth/merge` (`MergeAccounts { mode: move_login, confirm: true, code | credential }`) needs a fresh session and, when the absorbed account has factors, one of them (`POST /auth/merge/passkey/options` for its passkey). One transaction moves identities, passkeys, and verified emails (as non-primary), moves its events, repoints older tombstones, deletes the absorbed account (cascading its sessions, factors, and the app's rows), and writes `merged_accounts`; `Auth::resolve_merged` follows it. `POST /auth/merge/cancel` drops the offer.
- **Config:** `[auth.providers.<id>]` with `kind` (defaulting from the ID), `name`, `client_id_env`, `client_secret_env`, `issuer`, `tenant`, `team_id_env`, `key_id_env`, `private_key_env`, `scopes`; validated (IDs, required variables per kind, `https` issuers). `Auth::from_config` reads them; without the `oauth` feature, configured providers are a configuration error.
- **Client** (the same everywhere): `providers()`/`use_providers()`, `sign_in_with_provider()`, `link_provider()`, `continue_provider_sign_up()`, `provider_outcome()`, `identities()`/`use_identities()`, `unlink_identity()`, `merge_offer()`/`use_merge_offer()`, `merge_accounts()` (`MergeInput`), `cancel_merge()`. The browser's `Platform` navigates with `location.assign` and reads `location.search`; native apps answer `unsupported` for the three that need a browser.
- **CLI:** `leptoskit auth doctor [--release]` (variables set, never printed; keys parse; origin and RP ID; mail; `http` origins in `[native.plugin-options.http]` and `[auth] origin`, errors with `--release`; each provider's callback URL and definition age; App Store 4.8 when iOS has third-party sign-in without Apple), `leptoskit auth rotate-keys [--url]`, and `leptoskit auth provider add <id> [--issuer]` (writes the table with `toml_edit`, validates it, prints the setup steps with dev and `[auth] origin` callback URLs).
- **Docs:** the sign-in guide's new sections, and a page per provider under `docs/src/guide/auth/providers/`.
- **Tests:** `tests/passkeys.rs`, `tests/providers.rs`, `tests/provider_config.rs`, new cases in `tests/mfa.rs` and `tests/http.rs`, client tests with a software passkey, `oauth.rs` unit tests, and the CLI's `auth_command` tests. The fake identity provider (`tests/support/oidc.rs`) and the software passkey (`tests/support/passkey.rs`) make their keys at test time; no key material is committed.
- **Left for later:**
  - Auth 3: nothing in its list; the native system-browser flow is Auth 6, and "bring its data" merges are Auth 4.
  - Passkeys named from a bundled AAGUID list. (The merge preview's data counts arrived with Auth 4, below; the plan's `use_merge()` is `use_merge_offer()` plus `merge_accounts()`.)
  - `leptoskit build --release` refusing what `doctor --release` reports.
  - Turning `auth-passkeys` and `auth-providers` on in the `--auth` scaffold.

**As built (Auth 4: Solana wallets, and merging with data):**

- **Scalars:** `leptoskit_db`'s new `solana` feature adds `Pubkey` (32 bytes) and `Signature` (64 bytes), base58 text that must decode to exactly that many bytes, with `from_bytes`/`to_bytes` and `leptoskit_db::base58`. The decoding check needed a hook the scalar derive lacked: `#[db(validate_with = path)]` runs `fn(&Inner) -> Result<(), &'static str>` after the declared rules, recorded as the new `Rule::Format`. They landed here rather than in Auth 7, because linking needs them. `leptoskit_auth` always turns the feature on (the `wallets` table and the wire types use them); verification is behind its own `solana` feature.
- **SIWS** (`src/siws.rs`, built for every client): `SiwsInput` is the Wallet Standard's `solana:signIn` input (camelCase JSON, handed to wallets as is), `SiwsInput::message(address)` the one renderer, following `@solana/wallet-standard-util`'s `createSignInMessageText`, and `SiwsMessage::parse` a strict parser that accepts exactly what the renderer writes. Proptests: every rendered message parses back, whatever parses renders back byte for byte, single-character insertions and random bytes never slip past or panic. A `cargo fuzz` target isn't added.
- **Verification** (`src/wallets.rs`): the challenge is claimed first; the signed text must be byte-for-byte (constant time) the message rendered from the sealed fields for the signing address; it must parse; the message's expiry is checked again; then `ed25519-dalek` (already in the workspace, for code push) `verify_strict`. The domain is `[auth] origin`'s authority, the URI the origin, the chain `[auth.solana] chain_id` (default `mainnet`), the nonce 16 random bytes in base58, `Request ID` the purpose (`link`, `proof`, `sign-in`), the expiry five minutes. The plan's "account ID" isn't in the message: the challenge is bound to the session server-side, and the statement names the app.
- **Table:** `wallets` (`Wallet`): `(chain, address)` unique, `label`, `can_sign_in` (off), `is_public` (off), `verified_at`, `last_proved_at`, `proof` and `proof_signature` (the latest signed message, for audit). Owners read their own rows; `Auth::public_wallets` returns `PublicWallet { chain, address, label }` for those marked public, rather than a generated `Public*` type, so the account and proof never reach a profile. New purposes `WalletLink`, `WalletProof`, `WalletSignIn`; new events `WalletLinked`, `WalletUnlinked`, `WalletProved`, `WalletSignInChanged`, `WalletFailed`.
- **API:** `begin_wallet_link`/`finish_wallet_link` (fresh sign-in or step-up; `WalletLinked::{Linked, AlreadyLinked, Merge}`), `wallets`, `public_wallets`, `wallet_addresses`, `update_wallet` (label, `is_public`, `can_sign_in`, the last needing freshness and leaving a way to sign in), `unlink_wallet`, `begin_wallet_proof`/`finish_wallet_proof`, `require_wallet_proof(me, address, window)` (`ProofRequired`), `begin_wallet_sign_in`/`finish_wallet_sign_in` (`SignInStep`; `aal1`, then the second step when the account has factors). `sign_in_methods` counts wallets that may sign in, when the server allows it.
- **Decision: a wallet merges only where it signs in.** A wallet linked to another account makes a merge offer only when it may sign in there; otherwise `wallet_in_use`. A signature proves the key, and is only as strong as signing in to that account when that account chose to let the wallet sign in.
- **`cx.wallets()`:** `AuthContext::wallets() -> &[Pubkey]`, from a `Wallets` global. The layer now builds the context with `Auth::request_context`, one indexed query per signed-in request; `cx.wallets()` is empty unless a `SignedIn` is present too. `auth::context` still builds one without wallets.
- **Routes:** `GET /auth/wallets/config`, `GET /auth/wallets`, `POST /auth/wallets/{link,proof,sign-in}/options` and the three that finish them, `POST /auth/wallets/{update,unlink}`. The sign-in challenge's token goes in an `HttpOnly` `__Host-wallet` cookie (or the body for `bearer`), which the same-site fallback also refuses without an origin. Errors `wallet_failed` (400), `wallet_in_use` (409), `unknown_wallet` (404), `proof_required` (403), `merge_conflict` (409).
- **Config:** `[auth.solana]` with `sign_in` (default off) and `chain_id`; the facade's `auth-solana` turns on `leptoskit_auth/solana`.
- **Client** (the same everywhere): `wallet_settings()`/`use_wallet_settings()`, `wallet_apps()`/`use_wallet_apps()` (`WalletApp { name, icon, sign_in }`), `wallets()`/`use_wallets()`, `link_wallet()` (`LinkWalletInput`), `prove_wallet()` (`ProveWalletInput`), `update_wallet()`, `unlink_wallet()`, `wallet_sign_in()`. `Platform` gained `wallet_apps` and `sign_with_wallet`. The browser's (`client/wallet_standard.rs`) speaks the Wallet Standard's events without its JavaScript: it dispatches `wallet-standard:app-ready` and listens for `wallet-standard:register-wallet`, keeps wallets on a `solana:` chain, prefers `solana:signIn`, and falls back to `standard:connect` plus `solana:signMessage` over the rendered text, refusing a wallet that signed anything else; rejections (code 4001, or "reject"/"cancel") are `cancelled`. Native apps answer `unsupported` for the four calls that need a wallet before asking the server, and manage linked wallets normally. Server rendering reads `wallets()` and `wallet_settings()` in process.
- **Merging with data:** `#[db::table(on_account_merge = reassign | reassign_skip_conflicts | keep_survivor | keep_newest | custom)]` is stored in the schema model as `Table.on_account_merge`, which `without_rename_hints` strips, so snapshots and diffs never see it. `leptoskit_db_schema::account_data(schema, accounts)` finds tables keyed by the account (primary key = foreign key to it, transitively) and every other single-column foreign key to those, outside the accounts table's namespace, and reports problems: a unique reference without a strategy, `keep_newest` without one row per account and an `updated_at` column, or a keyed table with anything but `keep_survivor`/`custom`. `leptoskit_db::merge::{count_references, move_references}` run each strategy's SQL on both databases, on system connections only.
  - `OnMerge` (`summary`, `merge_profile`, `merge_custom`), registered with `AuthBuilder::on_merge::<U>()`. `MergeMode::BringData` is offered first when the schema is covered and every `custom` table has the hook. `MergeOffer` gained `wallets` and `data: Vec<MergeData { label, count, fate }>`; the default preview counts each reference's rows by table, merging a table's columns.
  - In the merge's transaction: every reference moves by its strategy, the `custom` tables and `merge_profile` run, then the login moves as before (wallets now included), and the account is deleted. Any error rolls everything back; the database's errors become `merge_conflict` with the table for the server's logs.
  - `leptoskit db check` now also fails, with sign-in's tables present, until every table is covered, and otherwise prints how many tables follow the account.
- **Screens:** Security gained "Wallets" (link with each wallet app, show/hide, allow/stop signing in, unlink; a note where the device has no wallets) and a merge preview with the mode choice and the data, worded per fate. Sign-in gained "Sign in with …" for each wallet app when the server allows wallet sign-in. Compiled and tested in a scratch crate on the host and for `wasm32` with `guest`; the class check and the `generate auth` snapshots cover them.
- **Tests:** `tests/wallets.rs` and `tests/merge.rs` (SQLite and Postgres), the client tests, `siws.rs` and `wallets.rs` unit and property tests, `leptoskit_db/tests/{solana,merge}.rs` (both databases), the schema analysis's unit tests, and the CLI's `db check` test. Test keys are made at test time.
- **Left for later:**
  - Native wallets: a native wallet adapter (Solana Mobile's Mobile Wallet Adapter on Android, deep-link wallets on iOS) behind the same `Platform` seam.
  - On-chain proof for multisigs, program-derived addresses, and hardware wallets (Auth 7, as planned).
  - A coverage-guided fuzz target for the SIWS parser.
  - Bringing data along for multi-column foreign keys and columns marked `no_foreign_key`, which the analysis can't see.
  - `keep_newest` for tables keyed by the account (they keep the survivor's row; `custom` covers the rest).

**As built (Auth 5, first layer: the provider catalog, and Facebook, X, Discord, LinkedIn, GitLab, and Slack):**

- **Providers are data.** `Provider` gained `Quirks`: how it identifies people (`Identify::IdToken(IdTokenRules)` or `Identify::Api(ApiProfile)`), how the app authenticates at the token endpoint (`ClientAuth::Form` or `Basic`), and whether linking sends `prompt=select_account` (only Google, Microsoft, GitHub, and generic OIDC, which are known to take it). `Flavor::GitHub` became `Flavor::Fixed`, the flavor of every definition with fixed endpoints. The new definitions live in `src/oauth/catalog.rs`; `catalog::built_in(kind, id, secret)` builds those that need only a client ID and secret.
- **`ApiProfile`** (`src/oauth/profiles.rs`) maps an API's JSON to a profile with JSON pointers (RFC 6901): subject (a string or a number, at most 255 bytes), name, username, picture, email, and a verification flag (`true`, `"true"`, or `1`; without a flag an address is never vouched for), or a separate address list whose primary entry counts (GitHub, now expressed this way too).
- **`IdTokenRules`:** `trust_email` (Microsoft's is false), `nonce_optional` (LinkedIn, whose ID tokens leave the nonce out; a nonce that's present must still match, and the code is still bound by `state` and the binding cookie), and `scope_claim` (Slack: the subject is `{https://slack.com/team_id}:{sub}`, and a token without the claim is refused). `IdClaims` keeps unknown claims in a flattened map for this.
- **Token requests:** with HTTP Basic, the client ID and secret are form-encoded first (RFC 6749 §2.3.1) and neither goes in the form. The token answer is read as JSON, and any non-null `error` (a string, or an object with a `message`, as Facebook sends) is a refusal.
- **The six:** Facebook (Graph API, version pinned by `FACEBOOK_GRAPH_VERSION` = `v26.0` or `[auth.providers.facebook] api_version`; no verification flag), X (OAuth 2 with PKCE, HTTP Basic, `/2/users/me`; no email), Discord (`/users/@me` and its `verified`), LinkedIn (OpenID Connect at fixed endpoints, issuer `https://www.linkedin.com/oauth`), GitLab (OpenID Connect at `GITLAB_COM` or a self-managed base given as `issuer`, which is also its issuer; a public client without a secret), and Slack (OpenID Connect, workspace-scoped subjects).
- **Config:** `leptoskit_config::AUTH_PROVIDER_KINDS` lists the built-in kinds; an ID that names one defaults to that kind. `issuer` is refused for built-in kinds other than `gitlab` (the message says to add `kind = "oidc"`), and `api_version` for anything but `facebook`, where it must look like `v26.0`. A built-in provider configured under another ID (`kind = "discord"` under `[auth.providers.chat]`) now keeps that ID; before, built-ins always took their kind's.
- **CLI:** `provider add` knows each new provider's console, what it calls the redirect URIs, and its notes; `facebook` writes `api_version`, and `gitlab --issuer` a self-managed base. `doctor` notes providers that never vouch for addresses and warns when Facebook's Graph API version is older than the pinned one.
- **Tests:** the fake identity provider (`tests/support/oidc.rs`) is now driven by a `Spec` per provider (its hosts, its issuer or none, whether it wants HTTP Basic, whether it echoes the nonce), refuses any other client authentication, and serves profile APIs from the person's `user` and `emails`. `tests/provider_catalog.rs` signs up and back in with each new provider on SQLite and Postgres, checks each authorization URL and scope, LinkedIn's nonce, Slack's workspaces, and profiles without a usable ID; `tests/provider_config.rs` builds the new kinds from config.
- **Docs:** a page per provider, and every provider page now says what native apps do (the system browser) instead of the stale `unsupported`.

**As built (Auth 5, second layer: Twitch, Spotify, Reddit, Amazon, and Dropbox):**

- **Two new quirks:** `Quirks.authorize`, extra parameters for the authorization page (Twitch's `claims` asking for `email`, `email_verified`, `picture`, and `preferred_username` in the ID token; Reddit's `duration=temporary`), and `Quirks.user_agent`, which sends `User-Agent: web:<app name>:leptoskit-auth/<version>` on the token and profile requests. The name comes from `[auth] app_name` and keeps only letters, digits, and `-_.` (spaces become dashes), so nothing about the person and no header injection can reach it. reqwest's request headers replace the client's default user agent.
- **The five:** Twitch (OpenID Connect at fixed endpoints, issuer `https://id.twitch.tv/oauth2`), Spotify (`/v1/me`, HTTP Basic; no verification flag), Reddit (`/api/v1/me` at `oauth.reddit.com`, HTTP Basic, the `identity` scope; no email), Amazon (`/user/profile`; no flag), and Dropbox (OpenID Connect, issuer `https://www.dropbox.com`, key set at `/.well-known/jwks`, all three from its discovery document).
- **Names from parts:** an ID token without `name` (Dropbox's carries only `given_name` and `family_name`) now gets one from those two.
- **CLI:** the five consoles; `doctor` notes that Spotify, Reddit, and Amazon never vouch for addresses, and names the app Reddit's requests will carry.
- **Tests:** the fake answers `429` to Reddit requests that don't name the app, checks Twitch's `claims` parameter, and serves the new hosts; `provider_catalog.rs` gained the five fixtures and their authorization URLs.

**As built (Auth 5, third layer: Bitbucket, Notion, Zoom, TikTok, and Yahoo):**

- **Four more quirks:** `client_id_param` (TikTok's `client_key`, in the authorization URL and the token form), `scope_separator` (TikTok's comma; with no scopes, as Notion has, `scope` is left out), `pkce` (off for Notion and TikTok's web sign-in, which document none and may refuse unknown parameters: there the code is bound by `state` and the binding cookie, and trading it needs the client secret), and `json_token_request` (Notion's token endpoint takes JSON, with HTTP Basic). The token request is now a list of fields encoded as a form or as JSON.
- **`ApiProfile.url` is optional:** without one, the token endpoint's answer is the profile (Notion names the grant's owner there). `ApiProfile.person` is a value the answer must hold to be a person; Notion's `/owner/user/type` must be `person`, so a grant owned by a bot or a workspace signs no one in.
- **The five:** Bitbucket (`/2.0/user`, subject `uuid`, HTTP Basic; addresses from `/2.0/user/emails`, whose primary entry counts when `is_confirmed`), Notion (`owner=user`, subject the owner's user ID, the same across workspaces; no verification flag), Zoom (`/v2/users/me`, HTTP Basic, `verified` of `1`), TikTok (Login Kit v2, `open_id`; no email), and Yahoo (OpenID Connect at fixed endpoints, issuer `https://api.login.yahoo.com`).
- **That completes the plan's 20 providers.** `doctor` notes Notion and TikTok as never vouching for addresses; `provider add` knows each console, and says TikTok accepts only `https` redirect URIs.
- **Tests:** the fake's `Spec` gained `pkce`, `client_param`, and `json`, and refuses a challenge or verifier sent to a provider without PKCE, or a form sent where JSON is expected; it answers profile-in-the-token providers from the person's `token` claims. `provider_catalog.rs` covers the five, and Notion grants owned by a workspace or a bot.

**As built (Auth 6: native sign-in):**

- **The flow is relayed, not redirected straight to the app.** The provider keeps sending the browser to the site's own `/auth/callback/{provider}`, so no provider needs a second redirect URI, and loopback redirects work for providers (Google's web clients) that refuse them. `POST /auth/oauth/{start,link}` takes `StartProviderRequest { start: StartProvider, native }`; the app-facing `StartProvider` is unchanged, and the client fills `native` in. `NativeStart::Browser { return_to }` answers `ProviderRedirect { url, token }` with the binding in the body and no cookie. The sealed `Flow` gained `native` (`#[serde(default)]`, so older rows open).
  - At the callback, `Auth::native_relay` finds the flow by `state` alone (purpose `Provider`, hash, live), and for a native flow answers `303` to `return_to` with the provider's `state`, `code`, `error`, and `user` appended, `Cache-Control: no-store`, `Referrer-Policy: no-referrer`. Nothing is used up, and no cookie or session is made: the code is worthless without the binding. Apple's cross-site `form_post` is relayed the same way, before the same-site check.
  - Why not finish at the callback: whoever opened the link would finish the flow of whoever started it. An attacker could start a flow, send the link, and redeem the victim's sign-in with their own binding. Relaying puts the code on the device that opened the link, where no binding for the attacker's flow exists.
  - `POST /auth/oauth/finish` (`FinishProvider`) runs `provider_profile` with `Mode::NativeBrowser`, then the same `finish_flow` as the web, as `Transport::Bearer` with the app's `device_name`. It answers `ProviderFinished { outcome, signed_in, second_factor }`: tokens when signed in, the second step's token when one is needed. `provider_profile` refuses a browser's flow in an app and an app's in a browser.
  - `POST /auth/oauth/continue` takes an optional `ContinueProvider { token, device_name }` body for native apps; browsers send none.
- **Return addresses** (`Auth::native_return`): the site's own `/auth/native/return` (`leptoskit_config::NATIVE_AUTH_RETURN_PATH`), `http://127.0.0.1:<port>` or `http://[::1]:<port>` with that path (RFC 8252 §7.3; never `localhost`, §8.3), or `<package id, lowercased>:/auth/native/return` (§7.1). No query, fragment, or credentials.
- **Native Sign in with Apple:** `NativeStart::AppleId` is refused (`unsupported`) unless the provider is Apple's and `package_id` is set; it answers a `nonce`. `finish` with `id_token` claims the flow by the binding alone (`take_ceremony` without a token), checks it's an `AppleId` flow, and verifies the token as the web flow does but with the bundle ID as audience. `verify_id_token`/`check_claims` take an `Expected { audience, nonce, now }`. The name comes from the device's `user` JSON, the shape Apple's web flow posts.
- **`NativeApps`** (`AuthBuilder::native_apps`, read by `from_config` from `[native]`): `package_id`, `apple_team_id`, and `android_certificates` (SHA-256 digests). The relying party appends `android:apk-key-hash:<base64url digest>` for each certificate, and `https://<rp id>` when there's a team and the site's origin has a port. `webauthn-rs` matches origins exactly, which is what makes the Android form work.
- **Config:** `[native] apple_team_id` (ten uppercase letters or digits) and `android_certificates` (`AB:CD:…`, colons optional), validated; `NativeConfig::android_certificate_digests`.
- **Well-known files:** `leptoskit_axum::app_associations::<S>(&NativeConfig)` (so `leptoskit::server::app_associations`) routes `/.well-known/apple-app-site-association` (`webcredentials.apps` and `applinks` for the return path, as `<team>.<package id>`) when there's a team, and `/.well-known/assetlinks.json` (`handle_all_urls` and `get_login_creds`, uppercase colon fingerprints) when there are certificates. The generated `sign_in.rs` gained `associations()`, and the `--auth` server merges it.
- **Plugins:**
  - `leptoskit_plugin_web_auth` (`web-auth`): `BEGIN` answers a session ID and return address; `AUTHENTICATE` takes the ID and an `https` page (or `http` on this device) and answers the address the browser came back to, checked against the return address. Swift: `ASWebAuthenticationSession` with a `customScheme` callback (the iOS 17.4/macOS 14.4 initializer when available). Kotlin: Auth Tab through the host's activity-result broker (`AuthTabIntent`'s intent with `EXTRA_REDIRECT_SCHEME`, `startActivityForResult`), `UNAVAILABLE` when `CustomTabsClient.isAuthTabSupported` says no. Rust `desktop`: a one-session loopback listener on `127.0.0.1:0`, the `open` crate, and a blocking `Plugin` for hosts that register it on a worker.
  - `leptoskit_plugin_passkeys` (`passkeys`): `CREATE`/`GET` carry WebAuthn's standard JSON (the options' `publicKey` object; `webauthn-rs-proto` reads the answers, including `clientExtensionResults`), and `APPLE_ID` takes a nonce and answers `AppleIdCredential`. Swift: `ASAuthorizationController` with platform passkey providers (`userVerificationPreference = .required`; excluded credentials from iOS 17.4) and `ASAuthorizationAppleIDProvider`. Kotlin: Credential Manager with the JSON as is; `APPLE_ID` is `UNAVAILABLE`.
- **Client:** the `Platform` seam gained `passkeys()`, `autofill()`, `begin_browser()`, `browse()`, `has_apple_id()`, and `apple_id()`, with defaults a browser never uses. The guest's `native::System` implements them over the two plugins. `browser_only` is gone: passkey calls check `passkeys()` (false without the plugin, so Windows, Linux, and HarmonyOS answer `unsupported` before asking the server), and `Autofill` checks `autofill()` (false in native apps). A native `start_provider` tries `AppleId` first when the passkeys plugin exists, falling back to the browser when the server says `unsupported` (another provider) or the plugin does (Android); then begins a browser session, starts, browses, checks the returned `state` against the sent one, and finishes. Failures after the browser opened become the outcome `Failed(code)`, as a browser's return page would show, and the mutation resolves; failures before it are errors. `provider_outcome()` reads the native outcome first. The `EmailInUse` binding is held in memory for `continue_provider_sign_up()`. `sign_in_with_provider()` now invalidates the session query on success, and `link_provider()` the identities and merge offer.
- **Screens:** the sign-in and security screens read `provider_outcome()` again when the provider mutation succeeds, which is when a native app is back; a browser still reads it on mount after the return navigation.
- **Scaffold:** `AUTH_PLUGINS` gained `passkeys` and `web-auth`, so `new --auth` and `generate auth` grant them and add both crates to the iOS, Android, and macOS hosts. The changed-files message lists them with a serial comma.
- **Tests:** `tests/native_providers.rs`, a native-origin case in `tests/passkeys.rs`, client tests with a fake system browser and Sign in with Apple (`Phone`), `leptoskit_axum/tests/associations.rs`, config validation, plugin codec and loopback tests, Swift tests for both plugins (macOS, plus an iOS simulator build), and Android instrumented tests (in `native_plugin_suite`) that never open a browser or a credential sheet. The ignored `the_auth_scaffold_signs_people_in` builds the scaffold's native guest with both plugins and loads it.
- **Left for later:**
  - The generated Xcode projects add no entitlements: the associated domain `webcredentials:<host>` (needed for passkeys) and Sign in with Apple are the app owner's to add.
  - Claimed `https` return addresses: the server accepts `<origin>/auth/native/return` and `applinks` claims it, but the plugins return to private-use schemes, which `ASWebAuthenticationSession` and Auth Tab deliver to the starting session only. Switching needs the entitlement above and verified Android app links.
  - Android browsers without Auth Tab: no Custom Tabs fallback (it would need a redirect intent filter in the generated manifest and a broker in `MainActivity`).
  - Windows and Linux: no native passkeys, and no system-browser passkey fallback page; the generated runners still host no plugins at all.
  - Passkey autofill in native text fields (`performAutoFillAssistedRequests`, Credential Manager's autofill).
  - `lk auth doctor` checking the association files against a deployed site.

**As built (IdP 1, first layer: `leptoskit_idp` tier 1):**

- **Crate and features:** `leptoskit_idp` with `server`, `postgres`, `sqlite` and `axum`, mirroring `leptoskit_auth`. The facade's `idp` feature follows `db-server`, `db-postgres`, `db-sqlite` and `server` the way `auth` does. Without features it has the tables and the wire types, which the CLI uses.
- **The `idp` pack:** `clients`, `grants` (unique per account and client), `authorizations` (a request waiting for consent, then a code, then used), `families`, `refresh_tokens` and `signing_keys`.
  - `grants` and `families` show the signed-in person their own rows; the rest deny every read. Everything cascades from `auth.accounts` and `idp.clients`.
  - **Packs may now link to other packs.** `leptoskit_db::schema()` validates a pack's tables together with every other pack's, and keeps only the problems in its own tables, so `idp` can reference `auth.accounts`. Linking to an app's table is still refused (database-plan §7).
- **Signing keys are derived, not stored.** A key's private half is `HMAC(AUTH_ENCRYPTION_KEYS[derived_from], "leptoskit derived key" ‖ "leptoskit_idp signing key " ‖ kid)`, through the new `Keys::derive`; the row keeps only `kid`, the server key's ID and the public coordinates. A copy of the database can't sign. Keys are created on first use, retire after 30 days, and stay in JWKS for the longest token lifetime plus an hour. A server key that's been removed (or replaced under the same ID, caught by comparing the derived public key) is skipped, and a new key is made.
- **ES256 only**, through `p256` 0.14 (RustCrypto, Apache-2.0 OR MIT), with a hand-written compact JWS: verification accepts only `ES256`, only a published `kid`, and only the expected `typ` (`at+jwt` for access tokens, `JWT` for ID tokens), so one kind can't pass as the other. No RS256, although OIDC Discovery says `RS256` must be listed; certification may need it.
- **Tokens (A4):** access tokens are 10-minute RFC 9068 JWTs (`iss`, `sub`, `aud` = the issuer unless `.audience()` says otherwise, `client_id`, `scope`, `auth_time`, `jti`, and `sid` = the refresh family). This server re-checks the family on every use, so revocation is immediate here; elsewhere it lags up to 10 minutes, as decided. Refresh tokens are 256 random bits stored as SHA-256, one family per code. Each works once, with **no grace period** (unlike first-party sessions): a second use, or a lost race between two uses, revokes the family. A replayed code revokes the family it started (RFC 6749 §4.1.2).
- **Pairwise subjects (A5):** `sub` is `SHA-256(label ‖ client.subject_salt ‖ account)`, with a random salt per client, so it's stable for the client's lifetime and doesn't depend on rotating keys. `first_party` clients see the account ID, and skip consent.
- **Client authentication:** `client_secret_basic`, `client_secret_post` and `none`, each held to the method it registered with. Secrets are 256 random bits, kept as SHA-256 and compared in constant time. `private_key_jwt` waits.
- **Redirects:** exact match, except loopback `http://127.0.0.1` and `[::1]` with any port (RFC 8252 §7.3). Registered URIs must be `https`, loopback `http`, or a reverse-domain scheme.
- **Authorize:** `response_type=code`, PKCE `S256` required, `response_mode=query`, `request`/`request_uri` refused. Responses carry `iss` (RFC 9207). `prompt=none|login|consent` and `max_age`; `prompt=login` returns through the sign-in page with `max_age=300` instead, so it can't loop. Errors that can't safely redirect (unknown client, unregistered redirect) are a small HTML page.
- **Consent is headless:** `/oauth/authorize` redirects to `consent_page?request=<handle>`; `GET /oauth/consent` returns `ConsentRequest`, `POST /oauth/consent` returns `ConsentRedirect { redirect_to }`. Handles are hashed, bound to the account, expire in 10 minutes and answer once. Consent is remembered per grant, as the union of approved scopes.
- **Two mount points, because of the same-site check.** `idp.router()` (authorize, consent, `end_session`, connected apps) goes inside `auth.layer()`, since it needs the cookie. `idp.layer()` goes outside it and serves discovery, JWKS, token, userinfo, revoke and introspect itself, with `Access-Control-Allow-Origin: *`: those read no cookies, and `auth.layer()` would refuse a cross-site `POST`.
- **Typed scopes on `#[api]`:** `leptoskit_rpc::scopes!` (a `macro_rules!`, not the plan's `#[derive(idp::Scopes)]`, so no new proc-macro crate) declares an enum with a `const fn as_str` and `impl Scopes`; doc comments become consent descriptions. `#[api(scope = …)]` (repeatable; a variant or a literal) records the names in `ApiFnDescriptor.scopes`, the spec's `ApiFunction.scopes`, and the OpenAPI document as `security: [{ oauth2: [...] }]` with an `oauth2` scheme.
  - **The trust boundary is the layer.** `idp.layer()` lets a JWT-shaped bearer token through only to a route `leptoskit_rpc::spec::scopes_for(method, path)` names with at least one scope, all of which the token carries; anything else is `403 insufficient_scope` (or `401 invalid_token`). Then it inserts `leptoskit_auth::Delegation`, and `auth.layer()` signs the request in as the consenting account (`SignedIn.session` is the nil ID, `aal1`), skips the same-site check, and exposes it on `CurrentSession.delegation`. The idp's own browser routes also refuse a delegated request.
- **Auth additions:** `Delegation`, `CurrentSession.delegation`, `Keys::derive` (keys derive from the one `Keys::newest` names), `Auth::keys()`, `Auth::origin()`, and a public `Auth::cleared_cookie()`.
- **Claims:** `Claims::claims(db, account, scopes, out)` fills a `ClaimSet`, with `db` acting as the person. The provider sets `sub`, and with `email` the verified primary address; reserved names can't be overwritten.
- **Logout:** `GET /oauth/end_session` with a valid `id_token_hint` (expired is fine) revokes that family, and ends the browser session too when it's the same account. Without a hint it signs nobody out. `post_logout_redirect_uri` must be registered.
- **Connected apps:** `GET /oauth/grants` and `POST /oauth/grants/revoke`. Disconnecting deletes the grant, which cascades to its families and refresh tokens, and removes untraded codes.
- **CLI:** `lk auth idp client add|list|remove`. The CLI builds the server and runs it with `LEPTOSKIT_IDP_COMMAND` (JSON) and `LEPTOSKIT_IDP_OUT` (a `0600` file in a private temporary directory); `Idp::run_command_if_requested()` answers and exits, like the RPC spec export.
- **Tests:** `crates/leptoskit_idp/tests/flow.rs` drives the real router (both layers, both routers) on SQLite and a temporary Postgres: discovery, sign-in redirect and return, consent (including another person's session), PKCE, wrong secrets, JWKS verification of the ID token, pairwise `sub`, `userinfo`, scoped and refused API calls, introspection, rotation, reuse detection, code replay, remembered consent, revocation, connected apps, public clients, `end_session`, `prompt` handling, malformed requests, first-party clients, and client management.
- **Left for the next layers:**
  - the OpenID conformance suite in CI (it needs the suite's Docker images and a reachable server, so this layer has only the offline flow test);
  - audit events for grants, consent and reuse (they need a way for another crate to write `auth.events`);
  - headless client hooks (`use_consent`, `use_connected_apps`) over the auth client's transport, and `lk generate auth --idp` pages;
  - `private_key_jwt`, RS256, `POST` for `/oauth/authorize` and `/oauth/end_session`, the `claims` request parameter, and `[auth.idp]` configuration in `leptoskit.toml` (everything is set on the builder today).

**As built (IdP 1 leftovers and IdP 2):**

- **Audit events from other crates:** `EventKind` gained `AppConnected`, `AppDisconnected`, `AppTokenReused`, `AppRegistered`, `AppChanged` and `AppDeleted`, and `auth.events` a `detail` column (at most 200 characters, never a credential) that names the app. `Auth::record(connection, account, kind, detail, request)` lets another crate write into the same trail inside its own transaction, which fits the pack rule that only `leptoskit_auth` owns its tables' schema. `Auth::limit_attempts(action, source, limit)` shares the rate limiter (keys are hashed apart from sign-in's), and `Auth::request_info(&request)` gives routes outside `auth.layer()` the same IP and user agent.
  - Recorded: a new grant or one that gained scopes (consent, first-party auto-consent, device approval); disconnecting, and an operator or developer removing an app (one `AppDisconnected` per person who had it); a replayed code, device code or refresh token, and a lost refresh race (once per family); portal registration, changes, secret rotation and deletion.
- **`Delegation.signed_in` is `Option<SignedIn>`.** `None` is an app acting as itself: the session layer leaves the request signed out, policies see no one, and `CurrentSession.delegation` names the app.
- **The headless client** (`leptoskit_idp::client`, features `client`/`web`/`guest`/`ssr`): `consent()`/`use_consent`, `decide_consent()`, `device_request()`/`use_device_request`, `decide_device()`, `connected_apps()`/`use_connected_apps`, `disconnect_app()`, `my_apps()`/`use_my_apps`, `register_app()`, `update_app()`, `rotate_app_secret()`, `delete_app()`.
  - **One transport:** `leptoskit_auth::client::send(method, path, body)` is new and public. It sends with the auth client's own credential holder (the cookie in the browser, the token keeper in a guest) and retries once after a `401` with renewal, so the idp hooks have no platform code at all. The idp client maps the `/oauth` JSON errors (`OAuthError`) onto `ErrorResponse`.
  - **SSR reads in process:** `idp.layer()` now inserts the `Idp` as a request extension; the `ssr` backend reads it and `leptoskit_auth::current_session()`, and refuses a delegated request, as the routes do. Changes answer `server`.
  - **Retries:** the read scopes retry (three times) only `network` and `server` failures; `not_found` and `login_required` stand.
  - The facade's `idp` turns on `leptoskit_idp/client`; `web`, `guest` and `server` follow with `leptoskit_idp?/web`, `?/guest` and `?/ssr`.
- **`lk generate auth --idp`** (one generator with the sign-in screens; the IdP stack first wrote a second `generate auth` in `crates/leptoskit_cli/src/generate/auth/`, removed when it was restacked onto the sign-in stack):
  - **Templates:** `auth_pages/templates/` holds the `--idp` version of every file; lines between `// leptoskit: idp` and `// leptoskit: end idp` are left out without the flag, and the markers never reach the app (`render`). The blocks are additive only, so the raw files compile as they are, and a run without `--idp` writes MFA's files byte for byte. A file whose `--idp` version isn't additive has a second template (`route_sign_in_idp.rs`).
  - **Screens:** `Consent` (`auth/consent.rs`), `DeviceCode` (`auth/device_code.rs`), `ConnectedApps` and `DeveloperApps`, in the `Copy`-state, `screen()` style, as fields of `Screens`. "Connected apps" is a tab in the account bar. The consent and device screens read their key from a signal that `open(…)` sets, from the route's `?request=` or `?user_code=`; their queries run only once opened and while signed in. The portal's list is read only once its screen is shown, so an app without the portal never asks for it. A typed device code is normalized (letters, digits and `-`, upper-cased) and opened in place, with no navigation.
  - **Leaving the app:** `Screens::navigate_with(f)` stores the app's `Navigate` (in the browser, `window.location.assign`); consent's answer goes to `redirect_to` through it, and says so on the screen if the app never set one. `/sign-in` with `--idp` takes `?return_to=` (`Screens::return_to`, kept only when `is_local_path`: starts with `/`, not `//`, and no `\`, whitespace or control characters, since browsers drop tabs and line breaks and `/\t/evil` would become `//evil`). An `Effect` continues only on a signed-out to signed-in transition seen on this device; someone already signed in gets "Continue" and "Sign in as someone else", so `prompt=login` (which returns through the sign-in page with `max_age=300`) can't loop.
  - **Routes:** `/consent`, `/device`, `/connected-apps` and `/developer-apps`, and the `--idp` `/sign-in`, which declares `SignIn { return_to }`, so the route becomes `Route::SignIn(SignIn)`.
  - **Wiring:** the `idp` feature on the client and the server, `[auth.idp]` appended when missing (checked to still load), and `sign_in::provider(&db, &auth)`, the provider's builder from the embedded config, in the server module. Next steps, each printed while its marker is missing: `navigate_with(` in the client entry, and `sign_in::provider` in `src/main.rs` (merge `idp.router()`, add `idp.layer()` outermost, add `leptoskit_idp` to `packs!`).
  - **Idempotence across modes:** a file still exactly as the other mode wrote it isn't the app's own. `--idp` replaces a file written without it ("added what --idp needs"); a run without `--idp` leaves an `--idp` file alone, so it never takes screens away. Any other difference is kept unless `--force`.
  - **Tests:** CLI unit tests for rendering, an `--idp` run, upgrading and not downgrading, and adding `[auth.idp]` to an existing `[auth]`; the Tailwind check covers every template. `examples/auth_pages` compiles the raw templates and route files through `#[path]` and renders each screen and route with `WidgetTester` over `auth::provide()`, with reads seeded into the cache (no server, so every change shows the client's error). It needs `reactive_graph/effects` and a `LocalSet`, since the screens rely on effects. The `--idp` server module was type-checked once against the facade's `server` and `db-sqlite` features; it isn't compiled in CI, because of its config-path placeholder.
- **`private_key_jwt`:** `ClientAuth::PrivateKeyJwt` with a registered JWK set (`clients.jwks`, at most 10 public ES256/RS256 keys; private members refused). An assertion must be signed by a registered key of its own algorithm (matched by `kid` when it has one), have `iss` = `sub` = the client, an `aud` of the issuer or one of the endpoints apps authenticate to, `exp` within an hour (60 s skew), and a `jti` that is recorded in `idp.assertions` (`SHA-256(client ‖ jti)`, unique) until it expires, so it works once. `typ` may be absent, `JWT` or `client-authentication+jwt`. Secrets and assertions can't be combined.
- **RS256:** only for ID tokens of clients registered with `id_token_alg = RS256`; access tokens stay ES256. The `rsa` crate is out: RUSTSEC-2023-0071 (Marvin) is unpatched in every release, and signing is network-observable. `ring` (already in the lockfile through rustls) signs and verifies, but can't generate RSA keys, so the operator supplies one (`IdpBuilder::rsa_key`, `[auth.idp] rsa_key_env`, PEM PKCS#8 or PKCS#1, at least 2048 bits). Its `kid` is the RFC 7638 thumbprint; `signing_keys` gained `algorithm`, optional `derived_from`, `x`/`y`, and `n`/`e`, and the RSA row is renewed while it signs, so a replaced key leaves JWKS after its last token expires. Discovery lists RS256 only when a key is configured. `jwt.rs` verifies only with the key's own algorithm.
- **Form `POST`s:** `idp.layer()` answers `POST /oauth/authorize` and `POST /oauth/end_session` (64 KiB, each parameter once) with a `303` to the `GET` route with the same parameters. A cross-site form post carries no `SameSite=Lax` cookie and fails the same-site check; the `GET` it's sent on to is a top-level navigation, which carries the cookie.
- **The `claims` parameter:** parsed and validated (at most 4 KiB; members `null` or objects with boolean `essential` and array `values`). `id_token` member names are stored on the authorization and family, and the ID token (also on refresh) gets those of the scope-released claims, with `email_verified` alongside `email`. `userinfo` members are accepted and ignored, since `userinfo` returns every released claim. `id_token.sub.value` that isn't the signed-in person's subject answers `login_required`. Discovery says `claims_parameter_supported: true`.
- **`[auth.idp]`:** `issuer`, `audience`, the four pages, `rsa_key_env`, `access_minutes` (≤ 60), `id_token_minutes`, `refresh_days`, `signing_key_days`, `developer_portal`, `apps_per_person`. `Idp::from_config(&db, &auth, &config)` returns the builder for the app's `scopes` and `claims`.
- **Grant types per client:** `clients.grant_types` (`authorization_code`, the device-code URN, `client_credentials`); refresh tokens come with the first two. A grant a client didn't register for is `unauthorized_client`; `/oauth/authorize` treats a client without the code grant as unknown. Redirect URIs are needed only for the code grant.
- **PAR:** `POST /oauth/par` (any client authentication, public clients too) runs every check `/oauth/authorize` would and keeps the request in `idp.pushed_requests` (handle hashed, 10 minutes by `Lifetimes::pushed_request`, longer than RFC 9126's examples so a sign-in fits). `/oauth/authorize?client_id&request_uri` uses only the pushed parameters, refuses another client's handle, and deletes it when the authorization is recorded. A signed-out visit keeps the handle in `return_to` instead of expanding it, and a pushed `prompt=login` is rewritten in place to `max_age=300`, as the plain flow does.
- **Device flow:** `POST /oauth/device_authorization` and the device-code grant; `idp.device_codes` keeps hashes of the device code and the normalized user code (8 letters from RFC 8628's 20 consonants, shown `XXXX-XXXX`; four tries on a clash). Polls compare-and-set `polled_at`; one sooner than `interval` is `slow_down` and adds 5 s. `GET`/`POST /oauth/device` (the person's own session only) count lookups at 20 per 15 minutes per account through `Auth::limit_attempts`, and answer `not_found` for anything not waiting. Approval records the grant; trading starts a family; a replayed device code ends it. Codes are kept a day past expiry for replay detection, and disconnecting an app deletes its waiting device codes.
- **Client credentials:** confidential clients only. Scopes default to the client's non-person scopes; `openid`, `profile`, `email` and `offline_access` are refused. Each token gets a family with no account or grant (`families.account_id` and `grant_id` are now optional), so `/oauth/revoke` and introspection work and verification re-checks it; machine families a day past expiry are deleted on the next issue. `sub` is the client ID. `rotate_secret` (CLI `client rotate-secret`, and the portal) revokes the client's machine families.
- **Developer portal:** `IdpBuilder::developer_portal(Portal { apps_per_person })`; the `/oauth/apps` routes are mounted only when it's on. `clients.owner_id` (cascading from the account) and a `select` policy show owners their own apps; `clients()` and `lk auth idp client list` show only operator apps. Portal apps are never first-party or introspecting and always ES256. The per-person limit is counted inside the insert's transaction. Removing scopes narrows every grant (deleting empty ones) and revokes families that carried a removed scope.
- **`lk login` for the app's own CLI:** the CLI generator adds `login` and `logout` (or `auth-login`/`auth-logout` if a function already has the name) when any function names a scope, with a generated `src/login.rs` from `crates/leptoskit_rpc_codegen/src/templates/cli_login.rs`. It discovers the endpoints, runs the device flow with every scope the spec names, and stores `credentials.json` (`0600`, written through a rename) under `$XDG_CONFIG_HOME`, `~/.config` or `%APPDATA%`. Each command attaches the access token and renews it 30 s early behind a `create_new` lock file (stale after 30 s), re-reading after the lock, since a second use of a refresh token ends the family. The client ID comes from `--client-id` or `<CLI>_CLIENT_ID`. `logout` revokes the refresh token and deletes the file. The OS keychain isn't used (no new dependency); that's a follow-up. An ignored test (`a_signing_in_cli_compiles`) builds the generated crate against the registry.
- **Tests:** `tests/flow.rs` and the new `tests/grants.rs` share `tests/support/mod.rs` and run on SQLite and a temporary Postgres; the crate's tables can only be written by the crate, so the tests move time with dialect-specific SQL through `Db::run_script`. `tests/fixtures/test-only-rsa-key.pem` is a 2048-bit key made for the tests.
- **OpenID conformance suite: not in CI yet, deliberately.** It can run on GitHub's Linux runners (build the suite from its GitLab source with Maven, `docker compose up` MongoDB and the Java server, and drive plans with `scripts/run-test-plan.py`), but it would fail, not flake:
  1. The basic and config OP plans send authorization requests without PKCE. This provider requires PKCE for every client (OAuth 2.1, decided in the plan's tier 1), so nearly every module would fail at `/oauth/authorize`. RFC 9700 §2.1.1 allows confidential OIDC clients that send `nonce` to skip PKCE; whether to offer that as a per-client opt-in (for certification and legacy RPs) is the owner's call.
  2. The suite drives sign-in and consent with HtmlUnit through configured page selectors, so it needs a purpose-built test server with an HTML sign-in page for a seeded user (email codes can't be automated) and an HTML consent page, plus an `https` issuer the suite's containers and browser both reach.
  3. This machine has no running container runtime, so a job written now couldn't be run before it lands.

  What exists instead: the in-process flows above exercise what those plans check (discovery, JWKS and both signature algorithms, code + PKCE, `state`/`nonce`/`iss`, `prompt` and `max_age`, `claims`, `userinfo` GET and POST, refresh rotation and reuse, code replay, revocation, introspection, end-session with `id_token_hint`, form posts, PAR, `private_key_jwt`). Once the PKCE question is decided, the next layer adds `examples/idp_conformance` (the test server) and a nightly, `workflow_dispatch` job that fails on any failed module.
- **Left for later:** the conformance job above; the generated CLI's tokens in the OS keychain; a `GET /auth/events` route and hook so a security page can show the activity without app code; RSA key rotation beyond "give the server a new one".

**As built (IdP 3: DPoP and dynamic registration):**

- **Both are off by default,** behind `[auth.idp] dpop` and `dynamic_registration` (with `registration_token_env`), or `IdpBuilder::dpop(true)` and `IdpBuilder::dynamic_registration(Registration { initial_access_token })`. Discovery lists `dpop_signing_alg_values_supported` and `registration_endpoint` only when they're on.
- **DPoP (RFC 9449), `src/dpop.rs`:**
  - A proof is checked for: one `DPoP` header; a compact JWS whose header has `typ: dpop+jwt`, `alg` ES256 or RS256, and a public `jwk` (private members refused) that verifies its own signature through `jwt.rs`, which now carries the header's `jwk`; `htm` equal to the method; `htu` equal to `issuer + path` by scheme, host, port (with defaults) and path, ignoring query and fragment; `iat` within 60 s either way; a `jti` of at most 256 bytes; and, with an access token, `ath` = base64url(SHA-256(token)), compared in constant time. The result is the key's RFC 7638 thumbprint (`jwt::thumbprint`, pinned to RFC 9449's own example).
  - **Replay:** `idp.dpop_proofs` keeps `SHA-256("dpop" ‖ jkt ‖ jti)`, unique, until `iat + 120 s`; expired rows go on the next insert, like `assertions`. A second use is `invalid_dpop_proof`. There are no server nonces: freshness is `iat` plus the replay table.
  - **Token endpoint:** with DPoP on, `http.rs` checks the proof for `POST /oauth/token` and sets `TokenRequest::dpop_jkt` (`#[serde(skip)]`, so a form can't). The access token gets `cnf.jkt` and the response `token_type: DPoP`. Every grant binds the access token; only a public app's family keeps the key (`families.jkt`), so its refresh tokens need a proof from the same key, checked before reuse detection so a thief without the key can't end the family. A confidential app's refresh tokens stay bound by its authentication, and it may refresh without a proof, getting a bearer token (RFC 9449 §5).
  - **`clients.dpop_bound`** (`dpop_bound_access_tokens`, `--dpop` on `client add`): the token endpoint refuses such an app without a proof. Registering one with DPoP off is `invalid_client_metadata`; turning DPoP off later leaves such apps refused, failing closed.
  - **Resources:** the layer and `userinfo` share `check_access`: a bound token works only as `Authorization: DPoP` with a proof for this method and URL, signed by the bound key; a bound token as `Bearer`, or an unbound one as `DPoP`, is `invalid_token`. Challenges use the token's scheme, so `WWW-Authenticate: DPoP error="…", algs="ES256 RS256"`. With DPoP off, the layer ignores the `DPoP` scheme as before. Introspection returns `token_type: DPoP` and `cnf`. CORS allows the `dpop` header.
  - **Left out:** `dpop_jkt` at `/oauth/authorize` and PAR (binding the code), `DPoP-Nonce`, and checking proofs at resource servers other than this one (they get `cnf` from introspection or the token).
- **Dynamic registration (RFC 7591 and 7592), `src/registration.rs`:**
  - `POST /oauth/register` (JSON `ClientMetadata`, `201 ClientRegistration`), and `GET`, `PUT`, `DELETE /oauth/register/{client_id}` with `Authorization: Bearer <registration_access_token>`. The layer serves them (cross-origin, no cookies) only when registration is on; otherwise the paths fall through to the app.
  - **Access:** with an initial access token (at least 32 characters, from `registration_token_env`), a request needs it, compared through SHA-256 in constant time; without one, registration is open and limited to 10 an hour per client IP through `Auth::limit_attempts`, counting refused attempts too, so probing costs the same. A wrong or missing token is `401 invalid_token` with `WWW-Authenticate: Bearer`.
  - **Mapping:** `token_endpoint_auth_method` defaults to `client_secret_basic`; `grant_types` to `authorization_code` (`refresh_token` is implied and accepted; `implicit`, `password` and others are refused); `response_types` must be `code` or empty, and `code` needs the code grant; `scope` defaults to `openid profile email`; `id_token_signed_response_alg` defaults to RS256 when the server has an RSA key (OIDC's default) and ES256 otherwise; `jwks_uri` is refused (inline `jwks` only); a bad redirect URI is `invalid_redirect_uri`; a missing `client_name` becomes the first redirect URI's host, or "Unnamed app". Informational fields (`logo_uri`, `contacts`, …) are ignored, and fields only the server sets are refused.
  - **The client:** `insert_client` with no owner and `clients.registration_hash` = SHA-256 of the registration access token; never first-party or introspecting; `ClientInfo.self_registered`, shown by `client list`. The answer shows the secret and the registration token once, with `client_secret_expires_at: 0`; reads never show the secret again (only its hash exists). `Debug` redacts both.
  - **Changes (`PUT`):** the body must carry the app's `client_id`, a `client_secret` only if it's the current one, and none of the server's fields. It replaces the metadata, narrows grants and ends families for removed scopes (the portal's `narrow_grants`), drops the secret when the app becomes public or key-authenticated, makes and returns a new one when it becomes secret-authenticated, and ends every family when the authentication method changes. The registration token isn't rotated.
  - **Delete:** `delete_client`, so everyone who connected it gets `AppDisconnected`.
- **Tests:** `tests/dpop.rs` and `tests/registration.rs` on SQLite and a temporary Postgres, plus unit tests for URL matching, thumbprints, redaction, and config. `tests/dpop.rs` makes its own proofs with `p256` and computes thumbprints independently of the provider.
- **Certification:** what it needs, and what still blocks it, is in [idp-certification.md](idp-certification.md). PKCE stays required for every client, as the owner decided, so the Basic, Config and Dynamic OP plans can't pass yet.

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
