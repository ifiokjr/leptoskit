//! `leptoskit-lint`: the leptoskit rules of `leptoskit lint`, without clippy
//! and without the rest of the CLI.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

/// Checks Rust files against the leptoskit project rules.
#[derive(Debug, Parser)]
#[command(name = "leptoskit-lint", version)]
struct Cli {
	/// Files or directories to check; the current directory by default.
	paths: Vec<PathBuf>,
	/// Fail on warnings as well as errors.
	#[arg(long)]
	deny_warnings: bool,
}

fn main() -> ExitCode {
	let cli = Cli::parse();
	let paths = if cli.paths.is_empty() {
		vec![PathBuf::from(".")]
	} else {
		cli.paths
	};
	let report = match leptoskit_lint::lint_paths(&paths) {
		Ok(report) => report,
		Err(error) => {
			let chain: Vec<String> =
				std::iter::successors(Some(&error as &dyn std::error::Error), |error| {
					error.source()
				})
				.map(ToString::to_string)
				.collect();
			let _ = writeln!(anstream::stderr(), "error: {}", chain.join(": "));
			return ExitCode::from(2);
		}
	};
	let base = std::env::current_dir().unwrap_or_default();
	let mut out = anstream::stdout().lock();
	for diagnostic in &report.diagnostics {
		let _ = writeln!(
			out,
			"{}",
			leptoskit_lint::render_diagnostic(diagnostic, &base)
		);
	}
	for path in &report.unparsed {
		let shown = path.strip_prefix(&base).unwrap_or(path);
		let _ = writeln!(out, "skipped {}: it does not parse", shown.display());
	}
	let plural = |count: usize, noun: &str| {
		if count == 1 {
			format!("{count} {noun}")
		} else {
			format!("{count} {noun}s")
		}
	};
	let _ = writeln!(
		out,
		"{} checked: {}, {}",
		plural(report.files, "file"),
		plural(report.warnings(), "warning"),
		plural(report.errors(), "error")
	);
	if report.failed(cli.deny_warnings) {
		ExitCode::FAILURE
	} else {
		ExitCode::SUCCESS
	}
}
