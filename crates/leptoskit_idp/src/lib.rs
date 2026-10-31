//! Sign in with your app: an OAuth 2.1 and OpenID Connect provider for
//! leptoskit apps.
//!
//! Other apps, including other leptoskit apps, send people to your app to
//! sign in, ask them for consent, and get tokens that call your `#[api]`
//! functions as them:
//!
//! - **The `idp` schema pack:** registered apps, consent grants, authorization
//!   codes, refresh-token families, and signing keys, as tables in the app's
//!   own database next to the `auth` pack. Add it with
//!   `leptoskit::db::packs!(leptoskit_auth, leptoskit_idp)`.
//! - **The protocol:** discovery, JWKS, the authorization code flow with PKCE
//!   `S256` only, rotating refresh tokens with reuse detection, `userinfo`,
//!   revocation, introspection, and RP-initiated logout.
//! - **Consent, headless:** `/oauth/authorize` sends people to your own consent
//!   page, which reads the request and posts the answer as JSON.
//! - **Typed scopes:** `#[api(scope = Scope::PostsRead)]` lets a third-party
//!   token call a function, which then runs as the person who consented, so
//!   access policies apply unchanged.
//!
//! The tables and the wire types work everywhere. The provider itself,
//! [`Idp`], needs the `server` feature and a database: `postgres` or
//! `sqlite`; its routes need `axum`.

mod api;
mod tables;

#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod authorize;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod clients;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod command;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod error;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod grants;
#[cfg(all(feature = "axum", any(feature = "postgres", feature = "sqlite")))]
mod http;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod idp;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod jwt;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod keys;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod secrets;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod tokens;

pub use api::*;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use authorize::AuthorizeOutcome;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use authorize::AuthorizeRequest;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use clients::ClientCredentials;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use error::IdpError;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use error::Result;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use grants::EndSession;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use grants::EndSessionRequest;
#[cfg(all(feature = "axum", any(feature = "postgres", feature = "sqlite")))]
pub use http::IdpLayer;
#[cfg(all(feature = "axum", any(feature = "postgres", feature = "sqlite")))]
pub use http::IdpService;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use idp::ClaimSet;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use idp::Claims;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use idp::GrantedScopes;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use idp::Idp;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use idp::IdpBuilder;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use idp::Lifetimes;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use leptoskit_rpc::ScopeInfo;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use leptoskit_rpc::Scopes;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use leptoskit_rpc::scopes;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use secrets::ClientSecret;
pub use tables::*;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use tokens::AccessToken;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use tokens::Introspection;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use tokens::TokenRequest;
