//! Diagnostics as text, laid out the way rustc and clippy lay out theirs so
//! the two read as one report.
//!
//! The text carries ANSI styles; write it through `anstream`, which strips
//! them when the output is not a terminal or `NO_COLOR` is set.

use std::fmt::Write as _;
use std::path::Path;

use anstyle::AnsiColor;
use anstyle::Style;
use leptoskit_config::LintLevel;

use crate::Diagnostic;

const TAB_WIDTH: usize = 4;

/// The style of each part of a diagnostic.
struct Palette {
	level: Style,
	gutter: Style,
	emphasis: Style,
}

impl Palette {
	fn for_level(level: LintLevel) -> Self {
		let color = match level {
			LintLevel::Deny => AnsiColor::BrightRed,
			LintLevel::Warn | LintLevel::Allow => AnsiColor::BrightYellow,
		};
		Self {
			level: Style::new().fg_color(Some(color.into())).bold(),
			gutter: Style::new()
				.fg_color(Some(AnsiColor::BrightBlue.into()))
				.bold(),
			emphasis: Style::new().bold(),
		}
	}
}

/// One diagnostic, with its source line and the span underlined. Paths are
/// shown relative to `base` when they are inside it.
#[must_use]
pub fn render_diagnostic(diagnostic: &Diagnostic, base: &Path) -> String {
	let palette = Palette::for_level(diagnostic.level);
	let Palette {
		level,
		gutter,
		emphasis,
	} = palette;
	let finding = &diagnostic.finding;
	let label = match diagnostic.level {
		LintLevel::Deny => "error",
		LintLevel::Warn | LintLevel::Allow => "warning",
	};
	// A path inside `base` is written with `/` on every platform, so output and
	// its snapshots read the same everywhere.
	let path = match diagnostic.path.strip_prefix(base) {
		Ok(relative) => {
			relative
				.components()
				.map(|component| component.as_os_str().to_string_lossy())
				.collect::<Vec<_>>()
				.join("/")
		}
		Err(_) => diagnostic.path.display().to_string(),
	};
	let line_number = finding.start.line.to_string();
	let pad = " ".repeat(line_number.len());

	let (indent, width) = underline(&diagnostic.source_line, finding);
	let source_line = diagnostic.source_line.replace('\t', &" ".repeat(TAB_WIDTH));

	let mut out = String::new();
	let _ = writeln!(
		out,
		"{level}{label}[leptoskit::{rule}]{level:#}{emphasis}: {message}{emphasis:#}",
		rule = finding.rule,
		message = finding.message,
	);
	let _ = writeln!(
		out,
		"{pad}{gutter}-->{gutter:#} {path}:{}:{}",
		finding.start.line,
		finding.start.column + 1,
	);
	let _ = writeln!(out, "{pad} {gutter}|{gutter:#}");
	let _ = writeln!(
		out,
		"{gutter}{line_number} |{gutter:#} {}",
		source_line.trim_end()
	);
	let _ = writeln!(
		out,
		"{pad} {gutter}|{gutter:#} {}{level}{} {}{level:#}",
		" ".repeat(indent),
		"^".repeat(width),
		finding.label,
	);
	let _ = writeln!(out, "{pad} {gutter}|{gutter:#}");
	let _ = writeln!(
		out,
		"{pad} {gutter}={gutter:#} {emphasis}help{emphasis:#}: {}",
		finding.help
	);
	out
}

/// The display column the underline starts at and how wide it is, with
/// tabs expanded as they are in the printed line.
fn underline(line: &str, finding: &crate::Finding) -> (usize, usize) {
	let display_width = |text: &str| -> usize {
		text.chars()
			.map(|character| if character == '\t' { TAB_WIDTH } else { 1 })
			.sum()
	};
	let before: String = line.chars().take(finding.start.column).collect();
	let width = if finding.end.line == finding.start.line {
		let spanned: String = line
			.chars()
			.skip(finding.start.column)
			.take(finding.end.column.saturating_sub(finding.start.column))
			.collect();
		display_width(&spanned)
	} else {
		1
	};
	(display_width(&before), width.max(1))
}
