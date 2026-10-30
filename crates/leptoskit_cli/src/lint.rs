//! `leptoskit lint`: the leptoskit project rules and clippy, run side by side
//! and reported as one.
//!
//! The project rules read source and finish in milliseconds, while clippy
//! compiles; clippy starts first and the rules run while it builds. Both
//! print in rustc's diagnostic layout, followed by one summary.

use std::io::BufRead as _;
use std::io::BufReader;
use std::io::Read as _;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::time::Instant;

use anyhow::Context as _;
use leptoskit_config::LintConfig;

use crate::ui;
use crate::ui::Outcome;

/// Checks the project with the leptoskit rules and clippy.
#[derive(Debug, clap::Args)]
pub struct LintArgs {
	/// Files or directories for the leptoskit rules; the whole project by
	/// default. clippy always checks the whole workspace.
	pub paths: Vec<PathBuf>,
	/// Run only the leptoskit rules.
	#[arg(long)]
	pub no_clippy: bool,
	/// Fail on warnings as well as errors.
	#[arg(long)]
	pub deny_warnings: bool,
	/// Arguments passed on to clippy, such as `-- -W clippy::nursery`.
	#[arg(last = true, value_name = "CLIPPY_ARGS")]
	pub clippy_args: Vec<String>,
}

/// What clippy reported.
#[derive(Debug, Default)]
struct ClippyReport {
	/// Each diagnostic, rendered by rustc.
	rendered: Vec<String>,
	warnings: usize,
	errors: usize,
	/// cargo's own output when it failed without a compiler diagnostic, such
	/// as an unreadable manifest.
	failure: Option<String>,
	elapsed: std::time::Duration,
}

/// Runs `leptoskit lint`.
///
/// # Errors
///
/// Returns an error when the project cannot be read, clippy cannot start,
/// or either check fails.
pub fn run(args: &LintArgs) -> anyhow::Result<()> {
	let cwd = std::env::current_dir().context("read the current directory")?;
	let root = leptoskit_config::LeptoskitConfig::find_root().unwrap_or_else(|_| cwd.clone());
	let config_path = root.join(leptoskit_config::CONFIG_FILE);
	let config = if config_path.is_file() {
		LintConfig::load_from_file(&config_path)?
	} else {
		LintConfig::default()
	};
	let paths = if args.paths.is_empty() {
		vec![root.clone()]
	} else {
		args.paths.clone()
	};
	let name = root.file_name().map_or_else(
		|| root.display().to_string(),
		|name| name.to_string_lossy().into_owned(),
	);
	ui::heading("lint", name);

	let clippy_wanted = config.clippy && !args.no_clippy;
	let clippy = clippy_wanted.then(|| {
		let root = root.clone();
		let extra = clippy_arguments(args);
		std::thread::spawn(move || run_clippy(&root, &extra))
	});

	let started = Instant::now();
	let report = leptoskit_lint::lint_paths(&paths)?;
	let rules_elapsed = started.elapsed();

	let clippy = match clippy {
		Some(handle) => {
			Some(
				handle
					.join()
					.map_err(|_| anyhow::anyhow!("the clippy thread panicked"))??,
			)
		}
		None => None,
	};
	// Everything prints once clippy is done, so the progress line it shows
	// while it builds never splits a diagnostic.
	let mut out = anstream::stdout().lock();
	for diagnostic in &report.diagnostics {
		let _ = writeln!(
			out,
			"{}",
			leptoskit_lint::render_diagnostic(diagnostic, &cwd)
		);
	}
	if let Some(clippy) = &clippy {
		for rendered in &clippy.rendered {
			let _ = writeln!(out, "{rendered}\n");
		}
		if let Some(failure) = &clippy.failure {
			let _ = writeln!(out, "{failure}\n");
		}
	}
	drop(out);

	let mut summary = ui::Summary::default();
	let mut rules_detail = vec![ui::count(report.files, "file")];
	rules_detail.push(
		problems(report.errors(), report.warnings()).unwrap_or_else(|| "no problems".to_owned()),
	);
	if !report.unparsed.is_empty() {
		rules_detail.push(format!(
			"{} not parsed",
			ui::count(report.unparsed.len(), "file")
		));
	}
	summary.row(
		outcome(report.errors(), report.warnings(), args.deny_warnings),
		"leptoskit",
		rules_detail.join(" · "),
		Some(rules_elapsed),
	);
	match &clippy {
		Some(clippy) if clippy.failure.is_some() => {
			summary.row(
				Outcome::Failed,
				"clippy",
				"cargo failed before clippy could check",
				Some(clippy.elapsed),
			);
		}
		Some(clippy) => {
			summary.row(
				outcome(clippy.errors, clippy.warnings, args.deny_warnings),
				"clippy",
				problems(clippy.errors, clippy.warnings)
					.unwrap_or_else(|| "no problems".to_owned()),
				Some(clippy.elapsed),
			);
		}
		None if args.no_clippy => {
			summary.row(Outcome::Skipped, "clippy", "skipped with --no-clippy", None);
		}
		None => {
			summary.row(
				Outcome::Skipped,
				"clippy",
				"skipped by `[lint] clippy = false`",
				None,
			);
		}
	}
	summary.print();

	anyhow::ensure!(!summary.failed(), "lint found problems to fix");
	Ok(())
}

/// The outcome of a check that found `errors` and `warnings`.
fn outcome(errors: usize, warnings: usize, deny_warnings: bool) -> Outcome {
	if errors > 0 || (deny_warnings && warnings > 0) {
		Outcome::Failed
	} else if warnings > 0 {
		Outcome::Warned
	} else {
		Outcome::Passed
	}
}

/// `1 error · 2 warnings`, or `None` when there are neither.
fn problems(errors: usize, warnings: usize) -> Option<String> {
	let parts: Vec<String> = [(errors, "error"), (warnings, "warning")]
		.into_iter()
		.filter(|&(count, _)| count > 0)
		.map(|(count, noun)| ui::count(count, noun))
		.collect();
	(!parts.is_empty()).then(|| parts.join(" · "))
}

/// What follows `--` on clippy's command line.
fn clippy_arguments(args: &LintArgs) -> Vec<String> {
	let mut extra = args.clippy_args.clone();
	if args.deny_warnings {
		extra.extend(["-D".to_owned(), "warnings".to_owned()]);
	}
	extra
}

/// Runs clippy over the workspace at `root`, collecting its diagnostics
/// from cargo's JSON messages and showing which crate it is on.
fn run_clippy(root: &Path, extra: &[String]) -> anyhow::Result<ClippyReport> {
	let started = Instant::now();
	let cargo = std::env::var_os("CARGO")
		.filter(|cargo| !cargo.is_empty())
		.unwrap_or_else(|| "cargo".into());
	let mut child = Command::new(cargo)
		.current_dir(root)
		.args([
			"clippy",
			"--workspace",
			"--all-targets",
			"--message-format=json-diagnostic-rendered-ansi",
			"--",
		])
		.args(extra)
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.context("start `cargo clippy`; is clippy installed? (`rustup component add clippy`)")?;

	// cargo's progress and errors go to stderr; read it alongside stdout so
	// neither pipe fills and stalls the build.
	let mut stderr = child.stderr.take().context("clippy's stderr")?;
	let stderr = std::thread::spawn(move || {
		let mut text = String::new();
		let _ = stderr.read_to_string(&mut text);
		text
	});

	let progress = ui::Progress::default();
	let mut report = ClippyReport::default();
	let mut seen = std::collections::HashSet::new();
	let stdout = child.stdout.take().context("clippy's stdout")?;
	for line in BufReader::new(stdout).lines() {
		let line = line.context("read clippy's output")?;
		let Ok(message) = serde_json::from_str::<serde_json::Value>(&line) else {
			continue;
		};
		match message["reason"].as_str() {
			Some("compiler-artifact") => {
				if let Some(name) = message["target"]["name"].as_str() {
					progress.update(format_args!("clippy · checking {name}"));
				}
			}
			Some("compiler-message") => {
				let diagnostic = &message["message"];
				let counted = match diagnostic["level"].as_str() {
					Some("warning") => &mut report.warnings,
					Some("error") => &mut report.errors,
					_ => continue,
				};
				// A diagnostic in code shared by several targets, such as a
				// library and its tests, arrives once per target.
				let rendered = diagnostic["rendered"]
					.as_str()
					.unwrap_or_default()
					.trim_end();
				if seen.insert(rendered.to_owned()) {
					*counted += 1;
					report.rendered.push(rendered.to_owned());
				}
			}
			_ => {}
		}
	}
	progress.finish();

	let status = child.wait().context("wait for clippy")?;
	let stderr = stderr.join().unwrap_or_default();
	if !status.success() && report.errors == 0 {
		report.failure = Some(stderr.trim_end().to_owned());
	}
	report.elapsed = started.elapsed();
	Ok(report)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn outcomes_follow_counts_and_the_warning_policy() {
		assert_eq!(outcome(0, 0, true), Outcome::Passed);
		assert_eq!(outcome(0, 2, false), Outcome::Warned);
		assert_eq!(outcome(0, 2, true), Outcome::Failed);
		assert_eq!(outcome(1, 0, false), Outcome::Failed);
	}

	#[test]
	fn problems_name_only_what_was_found() {
		assert_eq!(problems(0, 0), None);
		assert_eq!(problems(1, 0).as_deref(), Some("1 error"));
		assert_eq!(problems(2, 1).as_deref(), Some("2 errors · 1 warning"));
	}
}
