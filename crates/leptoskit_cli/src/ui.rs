//! How the CLI looks: one palette and a few shapes every command prints
//! with, so each reads as part of the same tool.
//!
//! Text is styled with ANSI escapes and written through [`anstream`], which
//! strips them when the stream is not a terminal, `NO_COLOR` is set, or the
//! terminal cannot show them. Commands never check for color themselves.

use std::fmt::Display;
use std::io::IsTerminal as _;
use std::io::Write as _;
use std::time::Duration;

use anstyle::AnsiColor;
use anstyle::Style;

/// The brand color, for the command name in headings.
pub const ACCENT: Style = Style::new()
	.fg_color(Some(anstyle::Color::Ansi(AnsiColor::BrightMagenta)))
	.bold();
/// Success marks.
pub const GOOD: Style = Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::BrightGreen)));
/// Warnings.
pub const WARN: Style = Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::BrightYellow)));
/// Errors.
pub const BAD: Style = Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::BrightRed)));
/// Secondary text: details, timings, paths.
pub const DIM: Style = Style::new().dimmed();
/// Names the reader should find at a glance.
pub const BOLD: Style = Style::new().bold();

/// How a step ended, which decides its mark and color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
	/// Nothing to report.
	Passed,
	/// Finished with warnings.
	Warned,
	/// Failed.
	Failed,
	/// Did not run.
	Skipped,
}

impl Outcome {
	fn mark(self) -> (&'static str, Style) {
		match self {
			Self::Passed => ("✔", GOOD),
			Self::Warned => ("▲", WARN),
			Self::Failed => ("✖", BAD),
			Self::Skipped => ("○", DIM),
		}
	}
}

/// Prints a failure to standard error: `error:` and the message, then each
/// cause on its own line, so a long chain reads top to bottom.
pub fn error(error: &anyhow::Error) {
	let mut err = anstream::stderr().lock();
	let bad = BAD.bold();
	let _ = writeln!(err, "{bad}error{bad:#}{BOLD}:{BOLD:#} {error}");

	for cause in error.chain().skip(1) {
		let _ = writeln!(err, "  {DIM}caused by:{DIM:#} {cause}");
	}
}

/// Prints something the user should know about but that did not stop the
/// command, to standard error.
pub fn warning(message: impl Display) {
	let warn = WARN.bold();
	let _ = writeln!(
		anstream::stderr().lock(),
		"{warn}warning{warn:#}{BOLD}:{BOLD:#} {message}"
	);
}

/// Prints a finished step: `✔ created notes in ./notes`.
pub fn success(message: impl Display) {
	let _ = writeln!(anstream::stdout().lock(), "{GOOD}✔{GOOD:#} {message}");
}

/// Prints a step that had nothing to do: `○ wasm client unchanged`.
pub fn skipped(message: impl Display) {
	let _ = writeln!(anstream::stdout().lock(), "{DIM}○ {message}{DIM:#}");
}

/// Prints a command to run, such as `next: cd notes && leptoskit dev`, with
/// the command in bold so it can be found and copied.
pub fn hint(label: &str, command: impl Display) {
	let _ = writeln!(
		anstream::stdout().lock(),
		"  {DIM}{label}:{DIM:#} {BOLD}{command}{BOLD:#}"
	);
}

/// The label a long-running command puts before each line from one of its
/// parts, such as `[web]` or `[ios]` in `leptoskit dev`, colored per part
/// so interleaved lines stay easy to follow.
#[must_use]
pub fn tag(part: impl Display) -> String {
	let part = part.to_string();
	const COLORS: [AnsiColor; 5] = [
		AnsiColor::BrightCyan,
		AnsiColor::BrightMagenta,
		AnsiColor::BrightBlue,
		AnsiColor::BrightGreen,
		AnsiColor::BrightYellow,
	];
	let index = part.bytes().map(usize::from).sum::<usize>() % COLORS.len();
	let style = Style::new()
		.fg_color(Some(anstyle::Color::Ansi(COLORS[index])))
		.bold();
	format!("{style}[{part}]{style:#}")
}

/// Prints a line from one part of a long-running command after its [`tag`]:
/// `[ios] booting iPhone 16`.
pub fn tagged(part: impl Display, message: impl Display) {
	let _ = writeln!(anstream::stdout().lock(), "{} {message}", tag(part));
}

/// Prints a part's failure to standard error after its [`tag`]:
/// `[android] skipping emulator-5554: it did not finish booting`.
pub fn tagged_error(part: impl Display, message: impl Display) {
	let _ = writeln!(anstream::stderr().lock(), "{} {message}", tag(part));
}

/// Prints the heading a command opens with: `leptoskit lint · harbor`.
pub fn heading(command: &str, detail: impl Display) {
	let mut out = anstream::stdout().lock();
	let _ = writeln!(
		out,
		"\n {ACCENT}leptoskit{ACCENT:#} {BOLD}{command}{BOLD:#} {DIM}·{DIM:#} {detail}\n"
	);
}

/// The table a command closes with: one row per step, its mark, name, what
/// happened, and how long it took, in aligned columns.
#[derive(Debug, Default)]
pub struct Summary {
	rows: Vec<SummaryRow>,
}

#[derive(Debug)]
struct SummaryRow {
	outcome: Outcome,
	name: String,
	detail: String,
	elapsed: Option<Duration>,
}

impl Summary {
	/// Adds a step; `elapsed` is `None` for a step that did not run.
	pub fn row(
		&mut self,
		outcome: Outcome,
		name: impl Into<String>,
		detail: impl Into<String>,
		elapsed: Option<Duration>,
	) {
		self.rows.push(SummaryRow {
			outcome,
			name: name.into(),
			detail: detail.into(),
			elapsed,
		});
	}

	/// Whether any step failed.
	#[must_use]
	pub fn failed(&self) -> bool {
		self.rows.iter().any(|row| row.outcome == Outcome::Failed)
	}

	/// Prints the table to standard output.
	pub fn print(&self) {
		let width = |text: &str| text.chars().count();
		let names = self
			.rows
			.iter()
			.map(|row| width(&row.name))
			.max()
			.unwrap_or(0);
		let details = self
			.rows
			.iter()
			.map(|row| width(&row.detail))
			.max()
			.unwrap_or(0);
		let mut out = anstream::stdout().lock();

		for row in &self.rows {
			let (mark, style) = row.outcome.mark();
			let name_pad = " ".repeat(names - width(&row.name));
			let detail_pad = " ".repeat(details - width(&row.detail));

			let elapsed = row.elapsed.map(seconds).unwrap_or_default();
			let _ = writeln!(
				out,
				"  {style}{mark}{style:#} {BOLD}{}{BOLD:#}{name_pad}  {}{detail_pad}  \
				 {DIM}{elapsed}{DIM:#}",
				row.name, row.detail,
			);
		}

		let _ = writeln!(out);
	}
}

/// A duration the way people say it: `850ms`, `4.2s`, `1m 05s`.
#[must_use]
pub fn seconds(elapsed: Duration) -> String {
	let millis = elapsed.as_millis();

	if millis < 1_000 {
		format!("{millis}ms")
	} else if millis < 60_000 {
		format!("{:.1}s", elapsed.as_secs_f64())
	} else {
		let seconds = elapsed.as_secs();
		format!("{}m {:02}s", seconds / 60, seconds % 60)
	}
}

/// Counts with their noun, pluralized: `1 warning`, `2 errors`.
#[must_use]
pub fn count(count: usize, noun: &str) -> String {
	if count == 1 {
		format!("{count} {noun}")
	} else {
		format!("{count} {noun}s")
	}
}

/// A line on standard error that rewrites itself while a long step runs,
/// and disappears when it is done. Silent when standard error is not a
/// terminal, so logs only hold the results. `Progress::default()` starts one.
pub struct Progress {
	live: bool,
}

impl Default for Progress {
	fn default() -> Self {
		Self {
			live: std::io::stderr().is_terminal(),
		}
	}
}

impl Progress {
	/// Replaces the line's text.
	pub fn update(&self, text: impl Display) {
		if self.live {
			redraw(&mut anstream::stderr().lock(), text);
		}
	}

	/// Clears the line.
	pub fn finish(&self) {
		if self.live {
			clear(&mut anstream::stderr().lock());
		}
	}
}

/// Returns to the start of the line, erases it, and writes `text` dimmed.
fn redraw(out: &mut impl std::io::Write, text: impl Display) {
	let _ = write!(out, "\r\x1b[2K  {DIM}{text}{DIM:#}");
	let _ = out.flush();
}

/// Returns to the start of the line and erases it.
fn clear(out: &mut impl std::io::Write) {
	let _ = write!(out, "\r\x1b[2K");
	let _ = out.flush();
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn durations_read_naturally() {
		assert_eq!(seconds(Duration::from_millis(850)), "850ms");
		assert_eq!(seconds(Duration::from_millis(4_240)), "4.2s");
		assert_eq!(seconds(Duration::from_secs(65)), "1m 05s");
	}

	#[test]
	fn counts_pluralize() {
		assert_eq!(count(0, "error"), "0 errors");
		assert_eq!(count(1, "warning"), "1 warning");
		assert_eq!(count(3, "file"), "3 files");
	}

	#[test]
	fn the_progress_line_rewrites_itself_in_place() {
		let mut out = Vec::new();
		redraw(&mut out, "checking leptoskit");
		clear(&mut out);
		let written = String::from_utf8(out).expect("utf-8");
		assert_eq!(
			written,
			format!("\r\x1b[2K  {DIM}checking leptoskit{DIM:#}\r\x1b[2K")
		);
	}
}
