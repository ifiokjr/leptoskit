//! The leptoskit CLI implementation library.
//!
//! The binary is a thin wrapper over [`cli::run`]; everything lives here so
//! tests can drive the command surface directly.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

pub mod build;
pub mod cache;
pub mod cli;
pub mod codepush;
pub mod compress;
pub mod dev;
pub mod doctor;
pub mod generate;
pub mod native;
pub mod new_project;
pub mod render;
pub mod report;
pub mod routes;
pub mod s3;
pub mod serve_cmd;
pub mod server;
pub mod testing;
pub mod wasm;
