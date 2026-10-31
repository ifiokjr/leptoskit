//! Sign-in for leptoskit apps.
//!
//! - **The `auth` schema pack:** accounts, verified emails, sessions,
//!   short-lived challenges, rate limits, and an audit trail, as tables in the
//!   app's own database. Add them to the crate that declares your tables with
//!   `leptoskit::db::packs!(leptoskit_auth)`.
//! - **Sessions:** opaque 256-bit tokens, of which the database keeps only a
//!   hash, with idle and absolute lifetimes and immediate revocation.
//! - **Email codes:** one flow that signs in to an address's account, or
//!   creates it, and never says which addresses have accounts.
//! - **Second factors:** an authenticator app (TOTP) with recovery codes, asked
//!   for after the email code once an account turns it on, and step-up, which
//!   sensitive actions ask for again. Passkeys count as both factors at once,
//!   and can be required for everyone or for certain roles.
//! - **Passkeys** (the `passkey` feature): WebAuthn on the web, through
//!   `webauthn-rs`, with ceremony state sealed on the server.
//! - **Third-party sign-in** (the `oauth` feature): Google, Apple, Microsoft,
//!   GitHub, and any OpenID Connect provider, identified by the provider's
//!   subject, never an email address; with linking, unlinking, and merging
//!   accounts.
//! - **Solana wallets** (the `solana` feature): linked to an account with a
//!   signed Sign-In With Solana message, re-proved when an action needs a fresh
//!   signature, and used to sign in only when their owner turns that on.
//! - **Merging accounts** in either mode: moving only the sign-in methods, or
//!   bringing the other account's data along, as each table's
//!   `on_account_merge` says.
//! - **[`SignedIn`]:** the signed-in person, as access policies read them:
//!   `cx.signed_in(|me| post.author_id.eq(me.user_id()))`, and their linked
//!   wallets, as `cx.wallets()`.
//!
//! The tables, [`SignedIn`], and the public row types work everywhere. The
//! service itself, `Auth`, needs the `server` feature and a database:
//! `postgres` or `sqlite`.

// The shared test helpers name this crate as an outside test would.
#[cfg(all(
	test,
	feature = "passkey",
	feature = "client",
	feature = "axum",
	feature = "sqlite"
))]
extern crate self as leptoskit_auth;

mod api;
#[cfg(feature = "client")]
pub mod client;
mod signed_in;
mod siws;
mod tables;

#[cfg(all(feature = "leptos", any(feature = "postgres", feature = "sqlite")))]
mod guards;
#[cfg(all(feature = "axum", any(feature = "postgres", feature = "sqlite")))]
mod http;

#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod auth;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod config;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod email;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod error;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod events;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod limits;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod mail;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod mailers;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod merge;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod mfa;
#[cfg(all(feature = "oauth", any(feature = "postgres", feature = "sqlite")))]
#[cfg_attr(
	not(feature = "axum"),
	allow(dead_code, reason = "the provider routes finish the flows")
)]
mod oauth;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
#[cfg_attr(
	not(all(feature = "axum", feature = "passkey")),
	allow(dead_code, reason = "the ceremonies and their routes use it")
)]
mod passkeys;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
#[cfg_attr(
	not(all(feature = "axum", feature = "oauth")),
	allow(dead_code, reason = "the provider routes finish the flows")
)]
mod providers;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod rotate;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod secrets;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod sessions;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod totp;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod wallets;

pub use api::*;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::Auth;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::AuthBuilder;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::CodeRules;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::MfaPolicy;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::NativeApps;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::OnSignUp;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::Profile;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::RequestInfo;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::Roles;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::SessionLifetimes;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::context;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use error::AuthError;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use error::Result;
#[cfg(all(feature = "leptos", any(feature = "postgres", feature = "sqlite")))]
pub use guards::*;
#[cfg(all(feature = "axum", any(feature = "postgres", feature = "sqlite")))]
pub use http::CurrentSession;
#[cfg(all(feature = "axum", any(feature = "postgres", feature = "sqlite")))]
pub use http::Delegation;
#[cfg(all(feature = "axum", any(feature = "postgres", feature = "sqlite")))]
pub use http::SessionLayer;
#[cfg(all(feature = "axum", any(feature = "postgres", feature = "sqlite")))]
pub use http::SessionService;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use limits::DbRateLimiter;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use limits::Limit;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use limits::LimitFuture;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use limits::Limits;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use limits::RateLimiter;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use mail::Mail;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use mail::MailError;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use mail::MailFuture;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use mail::Mailbox;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use mail::Mailer;
#[cfg(all(feature = "smtp", any(feature = "postgres", feature = "sqlite")))]
pub use mailers::SmtpMailer;
#[cfg(all(feature = "webhook", any(feature = "postgres", feature = "sqlite")))]
pub use mailers::WebhookMailer;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use merge::OnMerge;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use mfa::STEP_UP_WINDOW;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use mfa::SecondFactorChallenge;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use mfa::SignInStep;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use mfa::TotpConfirmed;
#[cfg(all(feature = "oauth", any(feature = "postgres", feature = "sqlite")))]
pub use oauth::Provider;
#[cfg(all(feature = "oauth", any(feature = "postgres", feature = "sqlite")))]
pub use oauth::ProviderFuture;
#[cfg(all(feature = "oauth", any(feature = "postgres", feature = "sqlite")))]
pub use oauth::ProviderHttp;
#[cfg(all(feature = "oauth", any(feature = "postgres", feature = "sqlite")))]
pub use oauth::ProviderRequest;
#[cfg(all(feature = "oauth", any(feature = "postgres", feature = "sqlite")))]
pub use oauth::ProviderResponse;
#[cfg(all(feature = "oauth", any(feature = "postgres", feature = "sqlite")))]
pub use oauth::ProviderStart;
#[cfg(all(feature = "oauth", any(feature = "postgres", feature = "sqlite")))]
pub use oauth::VERIFIED as PROVIDERS_VERIFIED;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use passkeys::PasskeyAdded;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use passkeys::PasskeyChallenge;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use providers::CallbackParams;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use providers::ProviderResult;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use rotate::Rotated;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use secrets::Keys;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use secrets::SessionToken;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use sessions::SignIn;
#[cfg(feature = "server")]
pub use signed_in::AuthContext;
pub use signed_in::SignedIn;
pub use signed_in::Wallets;
pub use siws::MAX_MESSAGE_LENGTH;
pub use siws::SiwsError;
pub use siws::SiwsInput;
pub use siws::SiwsMessage;
pub use tables::*;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use wallets::SolanaOptions;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use wallets::WalletSignInChallenge;
