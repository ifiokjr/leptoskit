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
pub mod db;
pub mod dev;
pub mod device;
pub mod devtools;
pub mod doctor;
pub mod extension;
pub mod generate;
pub mod l10n;
pub mod lint;
pub mod mcp;
pub mod migrate;
pub mod native;
pub mod new_project;
pub mod plugin_command;
pub mod publish;
pub mod render;
pub mod report;
pub mod routes;
pub mod rpc;
pub mod s3;
pub mod serve_cmd;
pub mod server;
pub mod skill;
pub mod split;
pub mod testing;
pub mod ui;
pub mod wasm;
