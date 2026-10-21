//! Formatting one source text: rustfmt for the Rust, the layout engine for
//! the markup, and passes until the two agree.
//!
//! A pass masks every markup macro (see [`mask_body`]), runs rustfmt once on
//! the masked file together with every expression the markup holds, lays the
//! markup out where rustfmt left each macro, and splices it back in. rustfmt
//! only ever sees masks, so its output depends on the markup only through
//! each mask's shape. When every macro prints to the same shape it was
//! masked as, a second pass would see the same input and change nothing: the
//! output is settled. A pass that changes a macro's shape (folding a short
//! macro onto one line, say) is followed by another.

use std::collections::HashSet;
use std::ops::Range;
use std::time::Duration;

use crate::Diagnostic;
use crate::config::Settings;
use crate::doc::Doc;
use crate::error::FormatError;
use crate::expr::Memo;
use crate::expr::Predict;
use crate::expr::Request;
use crate::expr::mask_body;
use crate::expr::mask_calls;
use crate::layout::Layouter;
use crate::layout::Parsed;
use crate::lexer::MacroCall;
use crate::lexer::find_macros;
use crate::markup;
use crate::markup::Markup;
use crate::markup::MarkupError;
use crate::rustfmt::Rustfmt;

/// More passes than any real file has needed; past this the output is
/// returned with a warning rather than looping.
const MAX_PASSES: usize = 4;

/// Rounds of rustfmt for expressions within one pass: one per level of
/// markup nested inside Rust inside markup.
const MAX_ROUNDS: usize = 16;

/// The result of formatting one source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
	/// The formatted text.
	pub text: String,
	/// Markup left as written, and why.
	pub diagnostics: Vec<Diagnostic>,
	/// Where the time went.
	pub stats: Stats,
}

/// Where the time formatting one file went.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
	/// Passes over the file: one when it was already formatted.
	pub passes: usize,
	/// rustfmt processes started.
	pub rustfmt_runs: usize,
	/// Time spent waiting for rustfmt; the rest is this crate's own work.
	pub rustfmt: Duration,
}

/// Formats `source` with `settings`, running `rustfmt`.
///
/// # Errors
///
/// Returns [`FormatError::Rustfmt`] when rustfmt rejects the file, which
/// usually means a syntax error.
pub fn format_source(
	source: &str,
	settings: &Settings,
	rustfmt: &Rustfmt,
) -> Result<Output, FormatError> {
	let output = format_passes(source, settings, rustfmt);
	// rstml's spans live in a thread-local source map that grows with every
	// parse; nothing outlives this call, so let it go.
	proc_macro2::extra::invalidate_current_thread_spans();
	output
}

fn format_passes(
	source: &str,
	settings: &Settings,
	rustfmt: &Rustfmt,
) -> Result<Output, FormatError> {
	let (bom, body) = match source.strip_prefix('\u{feff}') {
		Some(body) => ("\u{feff}", body),
		None => ("", source),
	};
	let crlf = settings.layout.newline_style.use_crlf(body);
	let mut text = body.replace("\r\n", "\n");
	let mut diagnostics = Vec::new();
	let mut stats = Stats::default();
	for _ in 0..MAX_PASSES {
		stats.passes += 1;
		let pass = Pass::run(&text, settings, rustfmt, &mut stats)?;
		diagnostics = pass.diagnostics;
		text = pass.text;
		if pass.settled {
			return Ok(finish(bom, &text, crlf, diagnostics, stats));
		}
	}
	diagnostics.push(Diagnostic {
		line: 1,
		column: 1,
		message: format!(
			"the layout did not settle after {MAX_PASSES} passes; formatting again may change it"
		),
	});
	Ok(finish(bom, &text, crlf, diagnostics, stats))
}

fn finish(bom: &str, text: &str, crlf: bool, diagnostics: Vec<Diagnostic>, stats: Stats) -> Output {
	let text = if crlf {
		text.replace('\n', "\r\n")
	} else {
		text.to_owned()
	};
	Output {
		text: format!("{bom}{text}"),
		diagnostics,
		stats,
	}
}

/// What rendering needs besides the macro itself.
struct Context<'a> {
	/// The pass's input, which the markup was parsed from.
	source: &'a str,
	/// rustfmt's output, which says where each macro goes.
	formatted: &'a str,
	settings: &'a Settings,
	rustfmt: &'a Rustfmt,
	stats: &'a mut Stats,
	parsed: &'a mut Parsed,
}

struct Pass {
	text: String,
	settled: bool,
	diagnostics: Vec<Diagnostic>,
}

impl Pass {
	fn run(
		source: &str,
		settings: &Settings,
		rustfmt: &Rustfmt,
		stats: &mut Stats,
	) -> Result<Self, FormatError> {
		let config = &settings.markup;
		let calls = find_macros(source, &config.macros);
		let markups: Vec<Result<Markup, MarkupError>> = calls
			.iter()
			.map(|call| markup::parse(source, call))
			.collect();

		// Lay the markup out once against the source to learn which
		// expressions it holds, so rustfmt formats them with the file.
		let mut predict = Predict::default();
		let mut parsed = Parsed::new();
		for (call, markup) in calls.iter().zip(&markups) {
			if let Ok(markup) = markup {
				let indent = settings
					.layout
					.width(line_indentation(source, call.range.start));
				Layouter::new(&settings.layout, config, source, &mut predict, &mut parsed)
					.macro_doc(call, markup, indent);
			}
		}
		let requests: Vec<Request> = predict.requests.into_iter().collect();
		let masked = mask_calls(source, &calls);
		let (formatted, answers) =
			rustfmt.file_and_expressions(&masked, &requests, settings, stats)?;
		let mut memo = Memo {
			answers,
			missing: HashSet::new(),
		};

		let placed = find_macros(&formatted, &config.macros);
		let moved = placed.len() != calls.len()
			|| placed
				.iter()
				.zip(&calls)
				.any(|(placed, call)| placed.path != call.path);
		if moved {
			return Err(FormatError::Rustfmt(
				"rustfmt added or removed a markup macro; the file was left unchanged".to_owned(),
			));
		}

		let mut pass = Self {
			text: String::with_capacity(formatted.len() + source.len() / 4),
			settled: true,
			diagnostics: Vec::new(),
		};
		let mut cursor = 0;
		for ((call, markup), place) in calls.iter().zip(&markups).zip(&placed) {
			pass.text.push_str(&formatted[cursor..place.range.start]);
			let rendered = match markup {
				Ok(markup) => {
					let context = Context {
						source,
						formatted: &formatted,
						settings,
						rustfmt,
						stats: &mut *stats,
						parsed: &mut parsed,
					};
					pass.render(call, markup, place, context, &mut memo)
				}
				Err(error) => {
					pass.diagnostics.push(diagnostic(source, error));
					source[call.range.clone()].to_owned()
				}
			};
			pass.text.push_str(&rendered);
			cursor = place.range.end;
		}
		pass.text.push_str(&formatted[cursor..]);
		Ok(pass)
	}

	/// Prints the markup of `call` where rustfmt placed it.
	fn render(
		&mut self,
		call: &MacroCall,
		markup: &Markup,
		place: &MacroCall,
		context: Context<'_>,
		memo: &mut Memo,
	) -> String {
		let Context {
			source,
			formatted,
			settings,
			rustfmt,
			stats,
			parsed,
		} = context;
		let layout = &settings.layout;
		let line = line_range(formatted, place.range.clone());
		let base = line_indentation(formatted, place.range.start);
		let start_column = layout.width(&formatted[line.start..place.range.start]);
		let suffix = &formatted[place.range.end..line.end];
		for _ in 0..MAX_ROUNDS {
			let mut layouter = Layouter::new(layout, &settings.markup, source, memo, parsed);
			let doc = layouter.macro_doc(call, markup, layout.width(base));
			let Layouter {
				builder,
				nested,
				failures,
				..
			} = layouter;
			if !memo.missing.is_empty() {
				let requests: Vec<Request> =
					std::mem::take(&mut memo.missing).into_iter().collect();
				memo.answers
					.extend(rustfmt.expressions(&requests, settings, stats));
				continue;
			}
			let suffix_doc = builder.text(suffix);
			let printed = builder.print(Doc::Concat(vec![doc, suffix_doc]), start_column, base);
			let rendered = printed.text[..printed.text.len() - suffix.len()].to_owned();
			let head = place.body.start - place.range.start;
			self.settled &=
				mask_body(&source[call.body.clone()]) == mask_body(body_of(&rendered, head));
			for nested in nested {
				let text = &printed.text[printed.tracks[nested.track].clone()];
				self.settled &= nested.mask == mask_body(body_of(text, nested.head));
			}
			self.diagnostics
				.extend(failures.iter().map(|error| diagnostic(source, error)));
			return rendered;
		}
		self.diagnostics.push(Diagnostic {
			line: line_number(source, call.range.start),
			column: 1,
			message: "the markup nests Rust and markup too deeply to format".to_owned(),
		});
		source[call.range.clone()].to_owned()
	}
}

/// The body of a printed macro call: between its head and closing
/// delimiter.
fn body_of(call: &str, head: usize) -> &str {
	call.get(head..call.len().saturating_sub(1))
		.unwrap_or_default()
}

/// The whitespace starting the line that contains `offset`.
fn line_indentation(text: &str, offset: usize) -> &str {
	let start = text[..offset].rfind('\n').map_or(0, |index| index + 1);
	let line = &text[start..];
	&line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// From the start of the line holding `range.start` to the end of the line
/// holding `range.end`, without the newline.
fn line_range(text: &str, range: Range<usize>) -> Range<usize> {
	let start = text[..range.start].rfind('\n').map_or(0, |index| index + 1);
	let end = text[range.end..]
		.find('\n')
		.map_or(text.len(), |index| range.end + index);
	start..end
}

fn line_number(text: &str, offset: usize) -> usize {
	text[..offset].matches('\n').count() + 1
}

fn diagnostic(source: &str, error: &MarkupError) -> Diagnostic {
	let offset = error.offset.min(source.len());
	let line_start = source[..offset].rfind('\n').map_or(0, |index| index + 1);
	Diagnostic {
		line: line_number(source, offset),
		column: source[line_start..offset].chars().count() + 1,
		message: format!("markup left as written: {}", error.message),
	}
}
