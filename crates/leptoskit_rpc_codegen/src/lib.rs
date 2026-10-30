//! The leptoskit server-function RPC specification, and the typed clients
//! generated from it.
//!
//! The specification is the contract between the two halves of
//! `leptoskit_rpc`:
//!
//! - the **server side** (the `leptoskit_rpc` crate) collects one
//!   [`ApiFunction`] per `#[leptoskit_rpc::api]` function at runtime — its
//!   route, HTTP encodings, documentation, and JSON Schemas for every argument
//!   and the return value — and serializes the [`ApiSpec`] this crate defines;
//! - the **tooling side** (the `leptoskit` CLI) reads that spec back and feeds
//!   it to the generators: typed clients that speak the RPC's own protocol —
//!   TypeScript over `fetch`, Python over the standard library, Go over
//!   `net/http`, Rust over `reqwest`, Dart over `package:http` — a command line
//!   client with one command per RPC method, and the OpenAPI 3.1 document for
//!   everything else.
//!
//! The specification is deliberately plain data — `serde` types over
//! `serde_json::Value` schemas — so it round-trips through a file or a URL
//! without linking the server binary against the generators.
//!
//! ```
//! # use leptoskit_rpc_codegen::ApiSpec;
//! # use leptoskit_rpc_codegen::GenerateOptions;
//! # use leptoskit_rpc_codegen::TargetLanguage;
//! let spec = ApiSpec::empty("demo", "0.1.0");
//! let generated = spec.generate(&GenerateOptions::new(TargetLanguage::OpenApi, "demo-api"));
//! assert_eq!(generated.files.len(), 2);
//! ```

pub mod openapi;
pub mod spec;
pub mod yaml;

#[cfg(feature = "generators")]
mod cli;
#[cfg(feature = "generators")]
mod dart;
#[cfg(feature = "generators")]
mod generate;
#[cfg(feature = "generators")]
mod go;
#[cfg(feature = "generators")]
mod naming;
#[cfg(feature = "generators")]
mod python;
#[cfg(feature = "generators")]
mod rust;
#[cfg(feature = "generators")]
mod schema;
#[cfg(feature = "generators")]
mod typescript;

#[cfg(feature = "generators")]
pub use generate::FileKind;
#[cfg(feature = "generators")]
pub use generate::GenerateOptions;
#[cfg(feature = "generators")]
pub use generate::Generated;
#[cfg(feature = "generators")]
pub use generate::GeneratedFile;
#[cfg(feature = "generators")]
pub use generate::Scaffold;
#[cfg(feature = "generators")]
pub use generate::TargetLanguage;
#[cfg(feature = "generators")]
pub use generate::Written;
pub use openapi::openapi_version;
pub use spec::ApiFunction;
pub use spec::ApiKind;
pub use spec::ApiParam;
pub use spec::ApiSpec;
pub use spec::CodegenError;
pub use spec::HttpInput;
pub use spec::HttpOutput;
pub use spec::SPEC_MARKER;
pub use spec::SPEC_VERSION;
