//! The `leptoskit format` command.

use std::io::IsTerminal as _;
use std::io::Read as _;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use crate::FormatError;
use crate::Rustfmt;
use crate::Settings;
use crate::format_source;
use crate::workspace;
use crate::workspace::Options;
use crate::workspace::Status;

/// Formats Rust files and the markup inside their `view!` macros.
#[derive(Debug, Clone, clap::Args)]
#[allow(
	clippy::struct_excessive_bools,
	reason = "each is an independent command-line flag"
)]
pub struct FormatArgs {
	/// Files or directories to format. Directories are walked respecting
	/// `.gitignore`. Defaults to the current directory.
	pub paths: Vec<PathBuf>,
	/// Write nothing; print a diff of every file that would change and fail
	/// if any would.
	#[arg(long)]
	pub check: bool,
	/// Format source read from stdin and write it to stdout.
	#[arg(long, conflicts_with = "paths")]
	pub stdin: bool,
	/// The path of the file read from stdin, used to find its configuration.
	#[arg(long, value_name = "PATH", requires = "stdin")]
	pub stdin_filepath: Option<PathBuf>,
	/// Skip files matching this glob when walking directories, such as
	/// `'**/fixtures/**'`. Repeatable.
	#[arg(long, value_name = "GLOB")]
	pub exclude: Vec<String>,
	/// Use stable rustfmt instead of nightly.
	#[arg(long, conflicts_with = "toolchain")]
	pub stable: bool,
	/// The rustup toolchain whose rustfmt to use, instead of the configured
	/// `rustfmt_toolchain` (nightly by default).
	#[arg(long, value_name = "TOOLCHAIN")]
	pub toolchain: Option<String>,
	/// Format every file, ignoring the record of files already formatted.
	#[arg(long)]
	pub no_cache: bool,
	/// Print each file, what happened to it, and where its time went.
	#[arg(long, short)]
	pub verbose: bool,
}

impl FormatArgs {
	fn toolchain(&self) -> Option<String> {
		if self.stable {
			Some("stable".to_owned())
		} else {
			self.toolchain.clone()
		}
	}
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
	/// Every file is formatted.
	Formatted,
	/// `--check` found this many files that would change.
	WouldFormat(usize),
	/// A file could not be formatted; the errors were printed.
	Failed,
}

impl Outcome {
	/// 0 when formatted, 1 when `--check` found changes, and 2 on failure.
	pub fn exit_code(self) -> ExitCode {
		match self {
			Self::Formatted => ExitCode::SUCCESS,
			Self::WouldFormat(_) => ExitCode::from(1),
			Self::Failed => ExitCode::from(2),
		}
	}
}

/// Runs the command, printing diffs, diagnostics, and errors.
#[must_use]
pub fn execute(args: &FormatArgs) -> Outcome {
	let result = if args.stdin {
		run_stdin(args)
	} else {
		run_files(args)
	};

	result.unwrap_or_else(|error| {
		eprintln!("error: {error}");
		Outcome::Failed
	})
}

/// Runs the command and returns its exit code.
pub fn run(args: &FormatArgs) -> ExitCode {
	execute(args).exit_code()
}

fn run_stdin(args: &FormatArgs) -> Result<Outcome, FormatError> {
	let mut source = String::new();
	std::io::stdin()
		.read_to_string(&mut source)
		.map_err(|source| {
			FormatError::Io {
				path: PathBuf::from("<stdin>"),
				source,
			}
		})?;

	let path = match &args.stdin_filepath {
		Some(path) => path.clone(),
		None => {
			std::env::current_dir()
				.map_or_else(|_| PathBuf::from("stdin.rs"), |dir| dir.join("stdin.rs"))
		}
	};

	let settings = Settings::for_path(&path)?;
	let toolchain = args
		.toolchain()
		.unwrap_or_else(|| settings.markup.rustfmt_toolchain.clone());
	// One file, so its expressions format alongside it.
	let rustfmt = Rustfmt::locate(&toolchain)?.concurrent(true);
	let started = Instant::now();
	let output = format_source(&source, &settings, &rustfmt)?;

	for diagnostic in &output.diagnostics {
		eprintln!("warning: {}:{diagnostic}", path.display());
	}

	if args.verbose {
		let stats = output.stats;
		eprintln!(
			"{:.2?} in total, {:.2?} in {} rustfmt runs over {} passes",
			started.elapsed(),
			stats.rustfmt,
			stats.rustfmt_runs,
			stats.passes
		);
	}

	if args.check {
		if output.text == source {
			return Ok(Outcome::Formatted);
		}

		print_diff(&path, &source, &output.text);

		return Ok(Outcome::WouldFormat(1));
	}

	std::io::stdout()
		.write_all(output.text.as_bytes())
		.map_err(|source| {
			FormatError::Io {
				path: PathBuf::from("<stdout>"),
				source,
			}
		})?;
	Ok(Outcome::Formatted)
}

fn run_files(args: &FormatArgs) -> Result<Outcome, FormatError> {
	let started = Instant::now();
	let paths = if args.paths.is_empty() {
		vec![PathBuf::from(".")]
	} else {
		args.paths.clone()
	};

	let files = workspace::collect_files(&paths, &args.exclude)?;
	let cache = if args.no_cache {
		None
	} else {
		std::env::current_dir()
			.ok()
			.and_then(|dir| workspace::default_cache_dir(&dir))
	};

	let options = Options {
		check: args.check,
		toolchain: args.toolchain(),
		cache,
		concurrent: files.len() == 1,
	};
	let report = workspace::format_files(&files, &options);

	for file in &report.files {
		for diagnostic in &file.diagnostics {
			eprintln!("warning: {}:{diagnostic}", file.path.display());
		}

		match &file.status {
			Status::Failed(error) => eprintln!("error: {}: {error}", file.path.display()),
			Status::Changed {
				original,
				formatted,
			} if args.check => print_diff(&file.path, original, formatted),
			_ => {}
		}

		if args.verbose {
			let status = match file.status {
				Status::Unchanged => "unchanged",
				Status::Cached => "cached",

				Status::Changed { .. } if args.check => "would format",
				Status::Changed { .. } => "formatted",
				Status::Failed(_) => "failed",
			};

			eprintln!(
				"{:>9.2?} {:>9.2?} rustfmt  {status:<12} {}",
				file.elapsed,
				file.stats.rustfmt,
				file.path.display()
			);
		}
	}

	let changed = report.changed().count();
	let failed = report.failed().count();
	let verb = if args.check {
		"would format"
	} else {
		"formatted"
	};
	let failures = if failed > 0 {
		format!("; {failed} failed")
	} else {
		String::new()
	};

	eprintln!(
		"{verb} {changed} of {} files in {:.2?}{failures}",
		report.files.len(),
		started.elapsed(),
	);
	Ok(if failed > 0 {
		Outcome::Failed
	} else if args.check && changed > 0 {
		Outcome::WouldFormat(changed)
	} else {
		Outcome::Formatted
	})
}

fn print_diff(path: &Path, original: &str, formatted: &str) {
	let diff = similar::TextDiff::from_lines(original, formatted);
	let display = path.display().to_string();
	let unified = diff
		.unified_diff()
		.context_radius(3)
		.header(&display, &display)
		.to_string();
	let colored = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
	let mut stdout = std::io::stdout().lock();

	for line in unified.lines() {
		let color = match line.as_bytes().first() {
			_ if !colored => None,

			Some(b'+') if !line.starts_with("+++") => Some("32"),

			Some(b'-') if !line.starts_with("---") => Some("31"),
			Some(b'@') => Some("36"),
			_ => None,
		};

		// A closed stdout (piping into `head`) is not worth failing for.
		let _ = match color {
			Some(color) => writeln!(stdout, "\x1b[{color}m{line}\x1b[0m"),
			None => writeln!(stdout, "{line}"),
		};
	}
}
