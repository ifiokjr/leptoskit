//! Bounded observability for an application, without a separate service.
//!
//! [`Analytics`] retains aggregate windows and a small correlation buffer.
//! With `server`, [`server::mount`] instruments Axum and provides protected
//! JSON evidence plus a same-origin browser SDK. The JSON API is `LeptosKit`'s
//! versioned format, not OTLP. Trace IDs use W3C Trace Context version 00.

mod aggregate;
mod context;
mod model;

pub use aggregate::Analytics;
pub use context::TraceContext;
pub use model::*;

#[cfg(feature = "server")]
pub mod export;
#[cfg(feature = "server")]
pub mod host;
#[cfg(feature = "server")]
pub mod server;
