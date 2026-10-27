//! `coverage_gate`: holds lcov tracefiles to the workspace floor and, given a
//! base revision, to the patch minimum. Tracefiles from several runs (the host
//! suite and each device) are merged before either gate is applied.
//!
//! Exits 0 when every gate passes, 1 when a gate fails, and 2 when the inputs
//! cannot be read.

use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitCode;

use coverage_gate::diff;
use coverage_gate::gate;
use coverage_gate::gate::Percent;
use coverage_gate::gate::Policy;
use coverage_gate::gate::Report;
use coverage_gate::lcov;
use coverage_gate::source;

const USAGE: &str = "\
usage: coverage_gate --lcov <file> [--lcov <file>...] --floor <file> [options]

Repeat --lcov to merge tracefiles from several runs; a line is covered when
any of them ran it.

options:
  --base <rev>           also gate the lines added since the merge base with <rev>
  --exclude-from <file>  path prefixes whose lines count neither way, one per
                         line, # comments allowed
  --patch-minimum <pct>  the share of added lines that must be covered (default 90)
  --markdown <file>      append the markdown report to <file>, such as $GITHUB_STEP_SUMMARY
  --annotate             print GitHub annotations for uncovered added lines
  --write-floor          set the floor to the measured total instead of checking it
";

struct Options {
	lcov: Vec<PathBuf>,
	floor: PathBuf,
	base: Option<String>,
	exclude_from: Option<PathBuf>,
	patch_minimum: Percent,
	markdown: Option<PathBuf>,
	annotate: bool,
	write_floor: bool,
}

impl Options {
	fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
		let mut lcov = Vec::new();
		let mut floor = None;
		let mut base = None;
		let mut patch_minimum = Percent::of(90, 100).expect("a nonzero total");
		let mut markdown = None;
		let mut exclude_from = None;
		let mut annotate = false;
		let mut write_floor = false;

		while let Some(flag) = args.next() {
			let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));
			match flag.as_str() {
				"--lcov" => lcov.push(PathBuf::from(value()?)),
				"--floor" => floor = Some(PathBuf::from(value()?)),
				"--exclude-from" => exclude_from = Some(PathBuf::from(value()?)),
				"--base" => base = Some(value()?),
				"--patch-minimum" => patch_minimum = value()?.parse()?,
				"--markdown" => markdown = Some(PathBuf::from(value()?)),
				"--annotate" => annotate = true,
				"--write-floor" => write_floor = true,
				other => return Err(format!("unknown argument `{other}`")),
			}
		}

		Ok(Self {
			lcov: if lcov.is_empty() {
				return Err("--lcov is required".into());
			} else {
				lcov
			},
			floor: floor.ok_or("--floor is required")?,
			exclude_from,
			base,
			patch_minimum,
			markdown,
			annotate,
			write_floor,
		})
	}
}

fn main() -> ExitCode {
	match run() {
		Ok(true) => ExitCode::SUCCESS,
		Ok(false) => ExitCode::FAILURE,
		Err(message) => {
			eprintln!("coverage_gate: {message}");
			ExitCode::from(2)
		}
	}
}

/// Whether every gate passed.
fn run() -> Result<bool, String> {
	let options =
		Options::parse(std::env::args().skip(1)).map_err(|error| format!("{error}\n\n{USAGE}"))?;
	let root = git(&["rev-parse", "--show-toplevel"])?;
	let mut coverage = lcov::Coverage::default();
	for path in &options.lcov {
		let tracefile = read(path)?;
		let run = lcov::parse(&tracefile, root.trim())
			.map_err(|error| format!("{}: {error}", path.display()))?;
		coverage.merge(run);
	}
	exclude_test_modules(&mut coverage, Path::new(root.trim()))?;
	let changes = match &options.base {
		Some(base) => {
			let diff = git(&[
				"-c",
				"core.quotePath=false",
				"diff",
				"--unified=0",
				"--no-color",
				"--no-ext-diff",
				"--no-textconv",
				"--find-renames",
				"--src-prefix=a/",
				"--dst-prefix=b/",
				&format!("{base}...HEAD"),
			])?;
			Some(diff::parse(&diff).map_err(|error| error.to_string())?)
		}
		None => None,
	};
	// Path prefixes whose lines count neither way: platform-tool glue no
	// measuring platform can run. One per line, # comments allowed.
	let excluded: Vec<String> = match &options.exclude_from {
		Some(path) => {
			fs::read_to_string(path)
				.map_err(|error| format!("reading {}: {error}", path.display()))?
				.lines()
				.map(str::trim)
				.filter(|line| !line.is_empty() && !line.starts_with('#'))
				.map(str::to_owned)
				.collect()
		}
		None => Vec::new(),
	};
	let report = Report::new(&coverage, changes.as_ref(), &excluded);
	let floor_file = read(&options.floor)?;

	if options.write_floor {
		let total = report
			.total
			.percent()
			.ok_or("the tracefile has no measured lines")?;
		let mut updated: String = floor_file
			.lines()
			.filter(|line| line.starts_with('#'))
			.flat_map(|line| [line, "\n"])
			.collect();
		updated.push_str(&total.to_decimal());
		updated.push('\n');
		fs::write(&options.floor, updated)
			.map_err(|error| format!("{}: {error}", options.floor.display()))?;
		println!("coverage floor set to {total}");
		return Ok(true);
	}

	let policy = Policy {
		floor: floor(&floor_file)
			.map_err(|error| format!("{}: {error}", options.floor.display()))?,
		patch_minimum: options.patch_minimum,
	};
	let markdown = report.markdown(policy);
	print!("{markdown}");
	if options.annotate {
		print!("{}", report.annotations());
	}
	if let Some(path) = &options.markdown {
		fs::OpenOptions::new()
			.create(true)
			.append(true)
			.open(path)
			.and_then(|mut file| file.write_all(markdown.as_bytes()))
			.map_err(|error| format!("{}: {error}", path.display()))?;
	}

	let failures = report.failures(policy);
	for failure in &failures {
		eprintln!("coverage_gate: {failure}");
	}
	Ok(failures.is_empty())
}

/// Drops the lines of `#[cfg(test)]` modules from every measured file, read
/// from the checkout at `root`: they are test code, and counting them would
/// let a change raise its own coverage by adding tests. A file the report
/// names but the checkout lacks (generated output) is left as it is.
fn exclude_test_modules(coverage: &mut lcov::Coverage, root: &Path) -> Result<(), String> {
	let paths: Vec<String> = coverage
		.files()
		.map(|(path, _)| path)
		.filter(|path| gate::is_measured(path))
		.map(str::to_owned)
		.collect();
	for path in paths {
		let source = match fs::read_to_string(root.join(&path)) {
			Ok(source) => source,
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
			Err(error) => return Err(format!("{path}: {error}")),
		};
		coverage.exclude(&path, &source::test_modules(&source));
	}
	Ok(())
}

/// The value of a floor file: its one line that is neither blank nor a `#`
/// comment.
fn floor(contents: &str) -> Result<Percent, String> {
	let mut values = contents
		.lines()
		.map(str::trim)
		.filter(|line| !line.is_empty() && !line.starts_with('#'));
	match (values.next(), values.next()) {
		(Some(value), None) => value.parse(),
		_ => Err("expected exactly one percentage".into()),
	}
}

fn read(path: &PathBuf) -> Result<String, String> {
	fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
}

fn git(args: &[&str]) -> Result<String, String> {
	let output = Command::new("git")
		.args(args)
		.output()
		.map_err(|error| format!("could not run git: {error}"))?;
	if !output.status.success() {
		return Err(format!(
			"`git {}` failed: {}",
			args.join(" "),
			String::from_utf8_lossy(&output.stderr).trim()
		));
	}
	String::from_utf8(output.stdout)
		.map_err(|_| format!("`git {}` printed invalid UTF-8", args.join(" ")))
}
