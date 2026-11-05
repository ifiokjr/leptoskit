//! <!-- {=cliSummary|trim|linePrefix:"//! ":true} -->
//!
//! `leptoskit verify build` builds a backend from a public Git commit in a
//! container without network access, twice, and records a bundle, its
//! inventory, and SLSA provenance; `leptoskit verify reproduce` lets anyone
//! rebuild that bundle and compare the source, dependencies, and executable
//! byte for byte. `verify record` and `verify check` inventory and check a
//! bundle offline. None of them verifies a running backend. See [checking a backend bundle](https://ifiokjr.github.io/leptoskit/guide/30-verifying-releases.html). Every workflow starts
//! here. The binary is a thin wrapper over the library, so the same code drives
//! `leptoskit dev`, CI builds, native host preparation, and release checks.
//! Generated projects call it by name; tooling calls the library directly.
//!
//! <!-- {/cliSummary} -->
//!
//! The binary is a thin wrapper over [`cli::run`]; everything lives here so
//! tests can drive the command surface directly.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

pub mod auth_command;
pub mod auth_pages;
pub mod build;
pub mod cache;
pub mod cargo_messages;
pub mod cli;
pub mod codepush;
pub mod compress;
pub mod console;
pub mod db;

pub mod deploy;
pub mod dev;
pub mod device;
pub mod devtools;
pub mod doctor;
pub mod extension;
pub mod generate;
mod git;
pub mod infra;

pub mod icons;
pub mod jobs;
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
pub mod verify;
pub mod wasm;

mod secrets;
