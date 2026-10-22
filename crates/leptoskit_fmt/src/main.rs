//! `leptoskit-fmt`: the `leptoskit format` command without the rest of the
//! CLI, for tooling that wants the formatter alone.

use std::process::ExitCode;

use clap::Parser;
use leptoskit_fmt::cli::FormatArgs;

/// Formats Rust files and the markup inside their `view!` macros.
#[derive(Debug, Parser)]
#[command(name = "leptoskit-fmt", version)]
struct Cli {
	#[command(flatten)]
	args: FormatArgs,
}

fn main() -> ExitCode {
	leptoskit_fmt::cli::run(&Cli::parse().args)
}
