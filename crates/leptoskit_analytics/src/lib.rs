//! <!-- {=analyticsSummary|trim|linePrefix:"//! ":true} -->
//!
//! See which application routes are failing or slow. The app keeps bounded
//! minute aggregates and a small correlation buffer in memory. Connect the
//! optional deployment service for durable fleet history, or use the
//! per-process endpoint without an extra observability service. Unexported
//! in-memory data resets on restart.
//!
//! <!-- {/analyticsSummary} -->
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
