//! Running rustfmt: once per file, with every expression inside the file's
//! markup batched into the same run.
//!
//! Starting rustfmt costs tens of milliseconds, far more than formatting the
//! markup, so the expressions are not formatted one by one. Each becomes the
//! value of a `let` inside a function appended to the file, nested in blocks
//! to the indentation it will have in the markup and with a binding name as
//! wide as the text before it on its line. One rustfmt run then formats the
//! file and every expression, each against the width it really has.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;

use crate::config::Layout;
use crate::config::Settings;
use crate::engine::Stats;
use crate::error::FormatError;
use crate::expr::Formatted;
use crate::expr::Request;
use crate::lexer::verbatim_lines;

/// The environment variable naming a rustfmt binary to use instead of
/// looking one up for the configured toolchain.
pub const RUSTFMT_ENV: &str = "LEPTOSKIT_RUSTFMT";

/// The functions holding expressions are named with this prefix.
const MARKER: &str = "__leptoskit_fmt_expression_";

/// A located rustfmt binary.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Rustfmt {
	program: PathBuf,
	/// `+toolchain` for rustup's proxy, when the binary could not be found
	/// directly.
	toolchain: Option<String>,
	/// Format a file's expressions in a second process alongside the file.
	concurrent: bool,
}

impl Rustfmt {
	/// A specific rustfmt binary.
	pub fn at(program: impl Into<PathBuf>) -> Self {
		Self {
			program: program.into(),
			toolchain: None,
			concurrent: false,
		}
	}

	/// Formats each file's markup expressions in a second rustfmt process,
	/// running alongside the one for the file instead of in the same run.
	///
	/// A file then waits for the slower of the two rather than for both, at
	/// the cost of starting another process. Worth it for one file, as when
	/// an editor formats on save; a run over many files keeps every core busy
	/// already, and spends less in total without it.
	#[must_use]
	pub fn concurrent(mut self, concurrent: bool) -> Self {
		self.concurrent = concurrent;
		self
	}

	/// Finds rustfmt for a rustup toolchain such as `nightly` or `stable`.
	///
	/// [`RUSTFMT_ENV`] wins when set, and an empty toolchain means whatever
	/// `rustfmt` is on the `PATH`. Otherwise the toolchain's own binary is
	/// used, found in rustup's toolchain directory, so each run skips the
	/// rustup proxy and its startup cost.
	///
	/// # Errors
	///
	/// Returns [`FormatError::RustfmtMissing`] when the toolchain has no
	/// rustfmt.
	pub fn locate(toolchain: &str) -> Result<Self, FormatError> {
		if let Some(program) = std::env::var_os(RUSTFMT_ENV) {
			return Ok(Self::at(program));
		}
		if toolchain.is_empty() {
			return Ok(Self::at("rustfmt"));
		}
		if let Some(program) = toolchain_binary(toolchain) {
			return Ok(Self::at(program));
		}
		let output = Command::new("rustup")
			.args(["which", "--toolchain", toolchain, "rustfmt"])
			.output();
		match output {
			Ok(output) if output.status.success() => {
				let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
				Ok(Self::at(path))
			}
			Ok(output) => {
				Err(FormatError::RustfmtMissing {
					toolchain: toolchain.to_owned(),
					message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
				})
			}
			// No rustup: the toolchain cannot be chosen, so use what is there.
			Err(_) => Ok(Self::at("rustfmt")),
		}
	}

	/// The binary this runs.
	#[must_use]
	pub fn program(&self) -> &Path {
		&self.program
	}

	/// What decides the output: the binary and the toolchain it runs.
	pub(crate) fn identity(&self) -> (&Path, Option<&str>) {
		(&self.program, self.toolchain.as_deref())
	}

	/// Formats `input` with the file's settings.
	pub(crate) fn run(
		&self,
		input: &str,
		settings: &Settings,
		stats: &mut Stats,
	) -> Result<String, String> {
		let started = std::time::Instant::now();
		let result = self.spawn(input, settings);
		stats.rustfmt_runs += 1;
		stats.rustfmt += started.elapsed();
		result
	}

	fn spawn(&self, input: &str, settings: &Settings) -> Result<String, String> {
		let mut command = Command::new(&self.program);
		if let Some(toolchain) = &self.toolchain {
			command.arg(format!("+{toolchain}"));
		}
		command
			.args(["--emit", "stdout", "--edition", &settings.edition])
			// The formatter converts line endings itself, after splicing the
			// markup back in.
			.args(["--config", "newline_style=Unix"]);
		if let Some(config) = &settings.rustfmt_config {
			command.arg("--config-path").arg(config);
		}
		if settings.directory.is_dir() {
			// Without a configuration file rustfmt searches from its working
			// directory, which must then be the file's.
			command.current_dir(&settings.directory);
		}
		let mut child = command
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.map_err(|error| format!("could not run {}: {error}", self.program.display()))?;
		let mut stdin = child.stdin.take().ok_or("rustfmt has no stdin")?;
		let input = input.to_owned();
		// Write from another thread: rustfmt may fill its stdout pipe before
		// it has read all of its input.
		let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
		let output = child
			.wait_with_output()
			.map_err(|error| format!("rustfmt did not finish: {error}"))?;
		let written = writer
			.join()
			.map_err(|_| "writing to rustfmt panicked".to_owned())?;
		// A failing rustfmt may stop reading early; its own message explains
		// more than the broken pipe that causes.
		if !output.status.success() {
			return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
		}
		written.map_err(|error| format!("could not write to rustfmt: {error}"))?;
		String::from_utf8(output.stdout)
			.map_err(|error| format!("rustfmt wrote invalid UTF-8: {error}"))
	}

	/// Formats `file` and `requests` in one run, falling back to separate runs
	/// to tell a broken file from an expression rustfmt cannot parse.
	pub(crate) fn file_and_expressions(
		&self,
		file: &str,
		requests: &[Request],
		settings: &Settings,
		stats: &mut Stats,
	) -> Result<(String, HashMap<Request, Arc<Formatted>>), FormatError> {
		if requests.is_empty() {
			let formatted = self
				.run(file, settings, stats)
				.map_err(FormatError::Rustfmt)?;
			return Ok((formatted, HashMap::new()));
		}
		if self.concurrent {
			return self.side_by_side(file, requests, settings, stats);
		}
		let synthetic = synthetic(requests, &settings.layout);
		let mut input = String::with_capacity(file.len() + synthetic.len() + 2);
		input.push_str(file);
		input.push_str("\n\n");
		input.push_str(&synthetic);
		if let Ok(output) = self.run(&input, settings, stats) {
			let start = output.rfind(&format!("\nfn {MARKER}0()")).ok_or_else(|| {
				FormatError::Rustfmt("rustfmt dropped the expressions it was given".to_owned())
			})?;
			let formatted_file = format!("{}\n", output[..start].trim_end());
			let answers = parse_synthetic(&output[start + 1..], requests, &settings.layout);
			return Ok((formatted_file, answers));
		}
		let formatted = self
			.run(file, settings, stats)
			.map_err(FormatError::Rustfmt)?;
		Ok((formatted, self.expressions(requests, settings, stats)))
	}

	/// Formats `file` and `requests` in two processes at once.
	fn side_by_side(
		&self,
		file: &str,
		requests: &[Request],
		settings: &Settings,
		stats: &mut Stats,
	) -> Result<(String, HashMap<Request, Arc<Formatted>>), FormatError> {
		let started = std::time::Instant::now();
		let mut file_stats = Stats::default();
		let mut expression_stats = Stats::default();
		let (formatted, answers) = std::thread::scope(|scope| {
			let expressions =
				scope.spawn(|| self.expressions(requests, settings, &mut expression_stats));
			let formatted = self.run(file, settings, &mut file_stats);
			(formatted, expressions.join())
		});
		stats.rustfmt_runs += file_stats.rustfmt_runs + expression_stats.rustfmt_runs;
		// The processes overlap, so the wait is the wall time, not the sum.
		stats.rustfmt += started.elapsed();
		let answers = answers
			.map_err(|_| FormatError::Rustfmt("formatting expressions panicked".to_owned()))?;
		Ok((formatted.map_err(FormatError::Rustfmt)?, answers))
	}

	/// Formats `requests` alone. An expression rustfmt cannot parse is kept
	/// as written.
	pub(crate) fn expressions(
		&self,
		requests: &[Request],
		settings: &Settings,
		stats: &mut Stats,
	) -> HashMap<Request, Arc<Formatted>> {
		let layout = &settings.layout;
		if let Ok(output) = self.run(&synthetic(requests, layout), settings, stats) {
			return parse_synthetic(&output, requests, layout);
		}
		if requests.len() == 1 {
			let request = requests[0].clone();
			let formatted = Arc::new(Formatted::unparsed(request.text.clone()));
			return HashMap::from([(request, formatted)]);
		}
		let (first, second) = requests.split_at(requests.len() / 2);
		let mut answers = self.expressions(first, settings, stats);
		answers.extend(self.expressions(second, settings, stats));
		answers
	}
}

/// The rustfmt binary of a rustup toolchain, when exactly one installed
/// toolchain directory matches this host.
fn toolchain_binary(toolchain: &str) -> Option<PathBuf> {
	let home = std::env::var_os("RUSTUP_HOME")
		.map(PathBuf::from)
		.or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".rustup")))?;
	let binary = if cfg!(windows) {
		"rustfmt.exe"
	} else {
		"rustfmt"
	};
	let prefix = format!("{toolchain}-{}-", std::env::consts::ARCH);
	let os = match std::env::consts::OS {
		"macos" => "apple-darwin",
		"windows" => "pc-windows",
		other => other,
	};
	let mut matches = std::fs::read_dir(home.join("toolchains"))
		.ok()?
		.filter_map(Result::ok)
		.filter(|entry| {
			entry
				.file_name()
				.to_str()
				.is_some_and(|name| name.starts_with(&prefix) && name.contains(os))
		})
		.map(|entry| entry.path().join("bin").join(binary))
		.filter(|path| path.is_file());
	let found = matches.next()?;
	matches.next().is_none().then_some(found)
}

/// The appended functions, one per request.
fn synthetic(requests: &[Request], layout: &Layout) -> String {
	let unit = layout.indent_unit();
	let mut out = String::new();
	for (index, request) in requests.iter().enumerate() {
		let (levels, extra) = nesting(request.indent, layout.tab_spaces);
		let _ = writeln!(out, "fn {MARKER}{index}() {{");
		for level in 1..levels {
			out.push_str(&unit.repeat(level));
			out.push_str("{\n");
		}
		let indentation = unit.repeat(levels);
		out.push_str(&indentation);
		let _ = write!(out, "let {} = ", binding(extra + request.prefix));
		let verbatim = verbatim_lines(&request.text);
		for (line_index, text_line) in request.text.split('\n').enumerate() {
			if line_index > 0 {
				out.push('\n');
				if !verbatim[line_index] && !text_line.is_empty() {
					out.push_str(&indentation);
				}
			}
			out.push_str(text_line);
		}
		out.push_str(";\n");
		for level in (1..levels).rev() {
			out.push_str(&unit.repeat(level));
			out.push_str("}\n");
		}
		out.push_str("}\n");
	}
	out
}

/// How many blocks deep an expression at `indent` columns sits, and the
/// columns left over.
fn nesting(indent: usize, tab_spaces: usize) -> (usize, usize) {
	let levels = (indent / tab_spaces.max(1)).max(1);
	(levels, indent.saturating_sub(levels * tab_spaces))
}

/// A binding whose `let NAME = ` is `prefix` columns wide: `let ` and ` = `
/// take seven, and the shortest pattern, `_`, one more.
fn binding(prefix: usize) -> String {
	let length = prefix.saturating_sub(7).max(1);
	format!("_{}", "x".repeat(length - 1))
}

/// Reads the formatted expressions back out of rustfmt's output.
fn parse_synthetic(
	output: &str,
	requests: &[Request],
	layout: &Layout,
) -> HashMap<Request, Arc<Formatted>> {
	let mut answers = HashMap::with_capacity(requests.len());
	let marker = format!("fn {MARKER}");
	for chunk in output.split(&marker).skip(1) {
		let Some((index, body)) = chunk.split_once("()") else {
			continue;
		};
		let Some(request) = index
			.parse::<usize>()
			.ok()
			.and_then(|index| requests.get(index))
		else {
			continue;
		};
		let formatted = extract(body, request, layout).unwrap_or_else(|| request.text.clone());
		answers.insert(request.clone(), Arc::new(Formatted::new(formatted)));
	}
	for request in requests {
		answers
			.entry(request.clone())
			.or_insert_with(|| Arc::new(Formatted::new(request.text.clone())));
	}
	answers
}

/// The expression bound in one formatted function, without the
/// indentation rustfmt gave it.
fn extract(body: &str, request: &Request, layout: &Layout) -> Option<String> {
	let (levels, extra) = nesting(request.indent, layout.tab_spaces);
	let name = binding(extra + request.prefix);
	let head = format!("let {name} =");
	let start = body.find(&head)? + head.len();
	let mut statement = &body[start..];
	// The end of the statement: the last `;` before the closing braces.
	let end = statement
		.trim_end()
		.trim_end_matches(|character: char| character == '}' || character.is_whitespace());
	statement = end.strip_suffix(';')?;
	let unit = layout.indent_unit();
	let mut depth = levels;
	let expression = if let Some(rest) = statement.strip_prefix(' ') {
		rest
	} else {
		// rustfmt moved the expression onto the next line, one level deeper.
		depth += 1;
		statement
			.trim_start_matches([' ', '\t'])
			.strip_prefix('\n')?
			.trim_start()
	};
	let indentation = unit.repeat(depth);
	let verbatim = verbatim_lines(expression);
	let lines: Vec<&str> = expression
		.split('\n')
		.zip(&verbatim)
		.enumerate()
		.map(|(index, (text_line, verbatim))| {
			if index == 0 || *verbatim {
				text_line
			} else {
				text_line
					.strip_prefix(indentation.as_str())
					.unwrap_or_else(|| text_line.trim_start())
			}
		})
		.collect();
	Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn layout() -> Layout {
		Layout {
			hard_tabs: true,
			..Layout::default()
		}
	}

	fn request(text: &str, indent: usize, prefix: usize) -> Request {
		Request {
			text: text.to_owned(),
			indent,
			prefix,
		}
	}

	#[test]
	fn nests_expressions_at_their_indentation() {
		let requests = [request("move || {\n\tx\n}", 8, 12)];
		assert_eq!(
			synthetic(&requests, &layout()),
			"fn __leptoskit_fmt_expression_0() {\n\t{\n\t\tlet _xxxx = move || \
			 {\n\t\t\tx\n\t\t};\n\t}\n}\n"
		);
	}

	#[test]
	fn keeps_string_lines_where_they_are() {
		let requests = [request("f(\"a\nb\")", 4, 1)];
		assert!(synthetic(&requests, &layout()).contains("f(\"a\nb\");"));
	}

	#[test]
	fn reads_expressions_back() {
		let requests = [request("x", 8, 1), request("y", 4, 1)];
		let output = "fn __leptoskit_fmt_expression_0() {\n\t{\n\t\tlet _ = move || \
		              {\n\t\t\tx\n\t\t};\n\t}\n}\nfn __leptoskit_fmt_expression_1() {\n\tlet _ \
		              =\n\t\tvery_long(\n\t\t\targument,\n\t\t);\n}\n";
		let answers = parse_synthetic(output, &requests, &layout());
		assert_eq!(answers[&requests[0]].text, "move || {\n\tx\n}");
		assert_eq!(answers[&requests[1]].text, "very_long(\n\targument,\n)");
	}

	#[test]
	fn falls_back_to_the_request_when_output_is_missing() {
		let requests = [request("x + 1", 4, 1)];
		let answers = parse_synthetic("", &requests, &layout());
		assert_eq!(answers[&requests[0]].text, "x + 1");
	}

	#[test]
	fn sizes_bindings_to_the_prefix() {
		assert_eq!(binding(0), "_");
		assert_eq!(binding(8), "_");
		assert_eq!(binding(12), "_xxxx");
		assert_eq!(nesting(10, 4), (2, 2));
		assert_eq!(nesting(0, 4), (1, 0));
	}
}
