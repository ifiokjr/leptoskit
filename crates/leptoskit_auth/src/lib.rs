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
//! - **[`SignedIn`]:** the signed-in person, as access policies read them:
//!   `cx.signed_in(|me| post.author_id.eq(me.user_id()))`.
//!
//! The tables, [`SignedIn`], and the public row types work everywhere. The
//! service itself, [`Auth`], needs the `server` feature and a database:
//! `postgres` or `sqlite`.

mod signed_in;
mod tables;

#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod auth;
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
mod secrets;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod sessions;

#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::Auth;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::AuthBuilder;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::CodeRules;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::OnSignUp;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::Profile;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::RequestInfo;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::SessionLifetimes;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use auth::context;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use error::AuthError;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use error::Result;
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
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use secrets::Keys;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use secrets::SessionToken;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use sessions::SignIn;
#[cfg(feature = "server")]
pub use signed_in::AuthContext;
pub use signed_in::SignedIn;
pub use tables::*;
