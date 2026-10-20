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
pub mod dev;
pub mod doctor;
pub mod generate;
pub mod new_project;
pub mod render;
pub mod serve_cmd;
pub mod server;
pub mod testing;
pub mod wasm;
