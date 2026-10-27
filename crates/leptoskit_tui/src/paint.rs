//! Painting the committed tree into the cell buffer.

use std::ops::Range;

use leptoskit_native::BorderWidths;
use leptoskit_native::FontWeight;
use leptoskit_native::NativeKind;
use leptoskit_native::Progress;
use leptoskit_native::TextAlign;
use leptoskit_native::TextSpan;
use leptoskit_yoga::NodeId;

use crate::backend::TuiBackend;
use crate::buffer::Buffer;
use crate::buffer::Rect;
use crate::cell::Attributes;
use crate::cell::TuiColor;
use crate::text::BUTTON_CHROME_CELLS;
use crate::text::CELL_HEIGHT;
use crate::text::CELL_WIDTH;
use crate::text::display_width;
use crate::text::wrap_ranges;

/// The braille spinner frames, one per tick.
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// The blocks an indeterminate progress bar sweeps with.
const SWEEP: usize = 3;

/// The border width, in logical pixels, from which edges draw heavy lines.
const HEAVY_BORDER: f32 = 2.0;

/// Paints the backend's committed tree into a fresh buffer.
pub(crate) fn paint(backend: &TuiBackend, width: u16, height: u16) -> Buffer {
	let mut buffer = Buffer::new(width, height);
	let Some(root) = backend.root() else {
		return buffer;
	};

	let mut painter = Painter {
		backend,
		buffer: &mut buffer,
		clip: Rect::new(0, 0, width, height),
	};

	painter.walk(root, 0.0, 0.0, Attributes::EMPTY, None);
	buffer
}

struct Painter<'a> {
	backend: &'a TuiBackend,
	buffer: &'a mut Buffer,
	clip: Rect,
}

impl Painter<'_> {
	/// Walks one node: `x` and `y` are the node's origin in viewport pixels,
	/// `dim` carries inherited dimness, and `underline` marks a focused
	/// `Pressable` subtree whose glyphs get the focus ring.
	fn walk(&mut self, id: NodeId, x: f32, y: f32, dim: Attributes, underline: Option<NodeId>) {
		let Some(node) = self.backend.node(id) else {
			return;
		};

		if node.props.hidden == Some(true) || node.props.opacity.unwrap_or(1.0) <= 0.0 {
			return;
		}

		let rect = cell_rect(x, y, node.frame.width, node.frame.height);
		if rect.is_empty() {
			return;
		}

		let dim = if node.props.enabled == Some(false) || node.props.opacity.unwrap_or(1.0) < 1.0 {
			dim.union(Attributes::DIM)
		} else {
			dim
		};

		let fg = TuiColor::from_packed(node.props.color.unwrap_or(transparent()));
		let bg = TuiColor::from_packed(node.props.background.unwrap_or(transparent()));
		let focused = self.backend.focused() == Some(id);
		let mut attrs =
			Attributes::for_weight(node.props.font_weight.unwrap_or(FontWeight::Regular))
				.union(dim);

		if node.props.italic == Some(true) {
			attrs = attrs.union(Attributes::ITALIC);
		}

		if underline.is_some() {
			attrs = attrs.union(Attributes::UNDERLINE);
		}

		match node.kind {
			NativeKind::View | NativeKind::Custom(_) => {
				self.container(rect, node, dim, id, x, y, underline, false);
			}
			NativeKind::ScrollView => {
				self.container(rect, node, dim, id, x, y, underline, true);
			}
			NativeKind::Text | NativeKind::TextRun => {
				self.container(rect, node, dim, id, x, y, underline, false);
				self.label(id, rect, fg, bg, attrs, false);
			}
			NativeKind::Button => {
				self.container(rect, node, dim, id, x, y, underline, false);
				self.button(id, rect, fg, bg, attrs, focused);
			}
			NativeKind::Pressable => {
				self.container(rect, node, dim, id, x, y, underline.or(Some(id)), false);
			}
			NativeKind::TextInput | NativeKind::PasswordInput => {
				self.container(rect, node, dim, id, x, y, underline, false);
				self.field(id, rect, fg, bg, attrs, focused);
			}
			NativeKind::Switch => self.mark(id, rect, fg, bg, attrs, focused, "switch"),
			NativeKind::Checkbox => self.mark(id, rect, fg, bg, attrs, focused, "checkbox"),
			NativeKind::Radio => self.mark(id, rect, fg, bg, attrs, focused, "radio"),
			NativeKind::Slider => self.slider(id, rect, fg, bg, attrs, focused),
			NativeKind::LinearProgressIndicator => {
				self.progress(id, rect, fg, bg, attrs);
			}
			NativeKind::ActivityIndicator => self.activity(id, rect, fg, bg, attrs),
			// Image and vector pixels, including Rive frames, do not resolve to cells.
			NativeKind::Image | NativeKind::Vector | NativeKind::Rive => {
				self.image(rect, bg, dim);
			}
		}
	}

	/// The shared container treatment: background, border, clipping, and
	/// recursion into children. A `Pressable` passes its focus ring down.
	#[allow(clippy::too_many_arguments)]
	fn container(
		&mut self,
		rect: Rect,
		node: &crate::backend::Node,
		dim: Attributes,
		id: NodeId,
		x: f32,
		y: f32,
		underline: Option<NodeId>,
		scroll: bool,
	) {
		self.background(rect, y, node);

		// Clipping bounds this subtree only; later siblings keep the outer
		// clip.
		let outer = self.clip;

		if node.props.clip == Some(true) || scroll {
			self.clip = self.clip.intersect(rect);
		}

		if scroll {
			let offset = self.backend.scroll(id).unwrap_or_default();
			self.scrolled_children(id, x, y, offset.x, offset.y, dim, underline);
		} else {
			self.children(id, x, y, dim, underline);
		}

		self.clip = outer;

		// The border draws over the children, as GTK paints it: a border
		// that takes no layout space still wins its edges.
		let edges = node.props.border_edges();

		if edges.any() {
			let border = TuiColor::from_packed(node.props.border_color.unwrap_or(transparent()));
			self.border(
				rect,
				Rows::of(y, node.frame.height),
				edges,
				border,
				dim,
				node.props.corner_radius.unwrap_or(0.0) > 0.0,
			);
		}
	}

	/// Paints a node's background. An opaque color fills its cells, with upper
	/// half blocks on a last row its height only half covers; a translucent one
	/// tints what is already painted, like a scrim.
	fn background(&mut self, rect: Rect, y: f32, node: &crate::backend::Node) {
		let Some(color) = node.props.background.filter(|color| color.alpha() > 0) else {
			return;
		};

		if color.alpha() < u8::MAX {
			self.buffer.tint(
				self.clip.intersect(rect),
				(color.red(), color.green(), color.blue()),
				color.alpha(),
			);
			return;
		}

		let bg = TuiColor::Rgb(color.red(), color.green(), color.blue());
		let rows = Rows::of(y, node.frame.height);
		let columns =
			|top: i16, height: i16| Rect::new(rect.left, top, rect.width, height.max(0) as u16);

		self.buffer
			.fill(self.clip.intersect(columns(rows.top, rows.full)), bg);

		if let Some(row) = rows.half_row() {
			self.buffer
				.half_fill(self.clip.intersect(columns(row, 1)), bg, true);
		}
	}

	fn children(&mut self, id: NodeId, x: f32, y: f32, dim: Attributes, underline: Option<NodeId>) {
		for &child in self.backend.children(id) {
			let Some(node) = self.backend.node(child) else {
				continue;
			};
			self.walk(child, x + node.frame.x, y + node.frame.y, dim, underline);
		}
	}

	#[allow(clippy::too_many_arguments)]
	fn scrolled_children(
		&mut self,
		id: NodeId,
		x: f32,
		y: f32,
		offset_x: f32,
		offset_y: f32,
		dim: Attributes,
		underline: Option<NodeId>,
	) {
		for &child in self.backend.children(id) {
			let Some(node) = self.backend.node(child) else {
				continue;
			};
			self.walk(
				child,
				x + node.frame.x - offset_x,
				y + node.frame.y - offset_y,
				dim,
				underline,
			);
		}
	}

	/// Writes a string of glyphs inside the clip.
	fn write(
		&mut self,
		text: &str,
		at_col: i16,
		row: i16,
		fg: TuiColor,
		bg: TuiColor,
		attrs: Attributes,
	) {
		let mut col = at_col;

		for symbol in text.chars() {
			// A no-break space only changes where lines wrap; on screen it is
			// an ordinary blank cell.
			let symbol = if symbol == '\u{a0}' { ' ' } else { symbol };

			if self.clip.contains(col, row) {
				self.buffer.set_glyph(col, row, symbol, fg, bg, attrs);
			}
			col += glyph_width(symbol);
		}
	}

	/// Draws the border edges that have a width: heavy lines from two pixels
	/// up, light ones below, and corners where two drawn edges meet, rounded
	/// when asked. A vertical edge ends in a half line (`╹`) on a row the box
	/// only half covers, matching its half-block background. Border glyphs keep
	/// the background under them.
	fn border(
		&mut self,
		rect: Rect,
		rows: Rows,
		edges: BorderWidths,
		color: TuiColor,
		dim: Attributes,
		rounded: bool,
	) {
		let right = rect.right() - 1;
		let top = rows.top;
		let Some(bottom) = rows.last().filter(|_| right >= rect.left) else {
			return;
		};

		let mut draw = |col: i16, row: i16, symbol: char| {
			if self.clip.contains(col, row) {
				self.buffer
					.set_glyph(col, row, symbol, color, TuiColor::Default, dim);
			}
		};

		for (width, row) in [(edges.top, top), (edges.bottom, bottom)] {
			if width > 0.0 {
				let line = if width >= HEAVY_BORDER { '━' } else { '─' };
				for col in rect.left..=right {
					draw(col, row, line);
				}
			}
		}

		for (width, col) in [(edges.left, rect.left), (edges.right, right)] {
			if width <= 0.0 {
				continue;
			}

			let heavy = width >= HEAVY_BORDER;
			for row in top..=bottom {
				let symbol = if rows.half_row() == Some(row) {
					if heavy { '╹' } else { '╵' }
				} else if heavy {
					'┃'
				} else {
					'│'
				};
				draw(col, row, symbol);
			}
		}

		let heavy = edges.top.max(edges.bottom).max(edges.left).max(edges.right) >= HEAVY_BORDER;
		let corners = match (heavy, rounded) {
			(true, _) => ['┏', '┓', '┗', '┛'],
			(false, true) => ['╭', '╮', '╰', '╯'],
			(false, false) => ['┌', '┐', '└', '┘'],
		};

		for (vertical, horizontal, col, row, symbol) in [
			(edges.left, edges.top, rect.left, top, corners[0]),
			(edges.right, edges.top, right, top, corners[1]),
			(edges.left, edges.bottom, rect.left, bottom, corners[2]),
			(edges.right, edges.bottom, right, bottom, corners[3]),
		] {
			if vertical > 0.0 && horizontal > 0.0 && right > rect.left && bottom > top {
				draw(col, row, symbol);
			}
		}
	}

	/// A wrapped, aligned label.
	fn label(
		&mut self,
		id: NodeId,
		rect: Rect,
		fg: TuiColor,
		bg: TuiColor,
		attrs: Attributes,
		center_by_default: bool,
	) {
		let Some(node) = self.backend.node(id) else {
			return;
		};
		let content = node.props.text.as_deref().unwrap_or_default();
		let max_cols = usize::from(rect.width);
		let lines = wrap_ranges(content, max_cols);
		let rtl = node.props.direction == Some(leptoskit_yoga::Direction::Rtl);
		// Runs paint only while they still spell the text that was measured.
		let spans = node.props.spans.as_deref().filter(|spans| {
			spans
				.iter()
				.flat_map(|span| span.text.chars())
				.eq(content.chars())
		});

		for (row, range) in lines.into_iter().enumerate() {
			let line = &content[range.clone()];
			let line_row = rect.top + row as i16;
			if line_row >= rect.bottom() {
				break;
			}

			let width = display_width(line);
			let start = match node.props.text_align.unwrap_or(if center_by_default {
				TextAlign::Center
			} else {
				TextAlign::Start
			}) {
				TextAlign::Start if rtl => rect.right() - width as i16,
				TextAlign::Start => rect.left,
				TextAlign::Center => rect.left + (rect.width as i16 - width as i16) / 2,
				TextAlign::End if rtl => rect.left,
				TextAlign::End => rect.right() - width as i16,
			};

			match spans {
				Some(spans) => self.write_spans(spans, range, start, line_row, fg, bg, attrs),
				None => self.write(line, start, line_row, fg, bg, attrs),
			}
		}
	}

	/// Writes one wrapped line of a `Text`'s styled runs: `range` indexes the
	/// runs joined together, and each glyph takes its own run's style over
	/// the label's.
	#[allow(clippy::too_many_arguments)]
	fn write_spans(
		&mut self,
		spans: &[TextSpan],
		range: Range<usize>,
		at_col: i16,
		row: i16,
		fg: TuiColor,
		bg: TuiColor,
		attrs: Attributes,
	) {
		let mut col = at_col;
		let mut span_start = 0;

		for span in spans {
			let span_end = span_start + span.text.len();
			let start = range.start.max(span_start);
			let end = range.end.min(span_end);

			if start < end {
				let fg = span.color.map_or(fg, TuiColor::from_packed);
				let bg = span.background.map_or(bg, TuiColor::from_packed);
				let mut attrs = match span.font_weight {
					Some(weight) => {
						attrs
							.without(Attributes::BOLD)
							.union(Attributes::for_weight(weight))
					}
					None => attrs,
				};
				attrs = match span.italic {
					Some(true) => attrs.union(Attributes::ITALIC),
					Some(false) => attrs.without(Attributes::ITALIC),
					None => attrs,
				};

				let text = &span.text[start - span_start..end - span_start];
				self.write(text, col, row, fg, bg, attrs);
				col += display_width(text) as i16;
			}

			span_start = span_end;
		}
	}

	/// A button: `[ label ]` with the app's own drawing when custom-drawn,
	/// reverse video when focused.
	fn button(
		&mut self,
		id: NodeId,
		rect: Rect,
		fg: TuiColor,
		bg: TuiColor,
		attrs: Attributes,
		focused: bool,
	) {
		let Some(node) = self.backend.node(id) else {
			return;
		};
		let label = node.props.text.as_deref().unwrap_or_default();
		let chrome = !node.props.is_custom_drawn();

		if !chrome {
			self.label(id, rect, fg, bg, attrs, true);
			return;
		}

		let mut line = String::with_capacity(label.len() + BUTTON_CHROME_CELLS);
		line.push('[');
		line.push(' ');
		line.push_str(label);
		line.push(' ');
		line.push(']');

		let width = display_width(&line) as i16;
		let align = node.props.text_align.unwrap_or(TextAlign::Center);
		let start = match align {
			TextAlign::Start => rect.left,
			TextAlign::Center => rect.left + (rect.width as i16 - width) / 2,
			TextAlign::End => rect.right() - width,
		};

		let attrs = if focused {
			attrs.union(Attributes::REVERSE)
		} else {
			attrs
		};

		self.write(&line, start, rect.top, fg, bg, attrs);
	}

	/// A field's value or placeholder with its block caret.
	fn field(
		&mut self,
		id: NodeId,
		rect: Rect,
		fg: TuiColor,
		bg: TuiColor,
		attrs: Attributes,
		focused: bool,
	) {
		let Some(node) = self.backend.node(id) else {
			return;
		};
		let draft = self.backend.draft(id);
		let value = draft.map_or_else(
			|| {
				node.props
					.value
					.as_deref()
					.or(node.props.text.as_deref())
					.unwrap_or_default()
					.to_owned()
			},
			|draft| draft.value.clone(),
		);

		if value.is_empty() {
			if let Some(placeholder) = node
				.props
				.placeholder
				.as_deref()
				.filter(|placeholder| !placeholder.is_empty())
			{
				self.write(
					placeholder,
					rect.left,
					rect.top,
					TuiColor::PLACEHOLDER,
					bg,
					attrs,
				);
			}
		} else if node.kind == NativeKind::PasswordInput {
			let masked: String = std::iter::repeat_n('•', value.chars().count()).collect();
			self.write(&masked, rect.left, rect.top, fg, bg, attrs);
		} else {
			self.write(&value, rect.left, rect.top, fg, bg, attrs);
		}

		// The caret is the terminal's own cursor, so it blinks in the user's
		// cursor style and input methods compose where the user types.
		if focused {
			let caret = draft.map_or(0, |draft| draft.caret) as i16;
			let col = (rect.left + caret).min(rect.right() - 1);

			if self.clip.contains(col, rect.top) {
				self.buffer
					.set_cursor(u16::try_from(col).ok().zip(u16::try_from(rect.top).ok()));
			}
		}
	}

	/// A boolean control's three-cell glyph.
	#[allow(clippy::too_many_arguments)]
	fn mark(
		&mut self,
		id: NodeId,
		rect: Rect,
		fg: TuiColor,
		bg: TuiColor,
		attrs: Attributes,
		focused: bool,
		kind: &str,
	) {
		let checked = self
			.backend
			.props(id)
			.and_then(|props| props.checked)
			.unwrap_or(false);

		let glyph = match (kind, checked) {
			("switch", true) => "[●]",
			("switch", false) => "[○]",
			("checkbox", true) => "[x]",
			("checkbox", false) => "[ ]",
			("radio", true) => "(x)",
			_ => "( )",
		};

		let attrs = if focused {
			attrs.union(Attributes::REVERSE)
		} else {
			attrs
		};

		self.write(glyph, rect.left, rect.top, fg, bg, attrs);
	}

	/// A slider's track and knob.
	fn slider(
		&mut self,
		id: NodeId,
		rect: Rect,
		fg: TuiColor,
		bg: TuiColor,
		attrs: Attributes,
		focused: bool,
	) {
		let state = self
			.backend
			.props(id)
			.and_then(|props| props.slider)
			.unwrap_or_default();
		let ratio = if state.max() > state.min() {
			((state.value() - state.min()) / (state.max() - state.min())).clamp(0.0, 1.0)
		} else {
			0.5
		};

		let knob = ((f64::from(rect.width) - 1.0) * ratio).round() as i16;
		let attrs = if focused {
			attrs.union(Attributes::REVERSE)
		} else {
			attrs
		};

		for col in rect.left..rect.right() {
			let symbol = if col - rect.left == knob {
				'●'
			} else {
				'─'
			};
			if self.clip.contains(col, rect.top) {
				self.buffer.set_glyph(col, rect.top, symbol, fg, bg, attrs);
			}
		}
	}

	/// A progress bar: filled to its fraction, or sweeping when
	/// indeterminate.
	fn progress(&mut self, id: NodeId, rect: Rect, fg: TuiColor, bg: TuiColor, attrs: Attributes) {
		if let Some(Progress::Determinate(value)) = self
			.backend
			.props(id)
			.map(|props| props.progress.unwrap_or_default())
		{
			let filled = (value.get() * f64::from(rect.width)).round() as i16;

			for col in rect.left..rect.right() {
				let symbol = if col - rect.left < filled {
					'█'
				} else {
					'░'
				};
				if self.clip.contains(col, rect.top) {
					self.buffer.set_glyph(col, rect.top, symbol, fg, bg, attrs);
				}
			}
		} else {
			let width = rect.width as i16;
			let head = (self.backend.tick() as i16) % (width + SWEEP as i16);

			for col in rect.left..rect.right() {
				let offset = col - rect.left;
				let in_sweep = ((head - offset).rem_euclid(width + SWEEP as i16)) < SWEEP as i16;
				let symbol = if in_sweep { '█' } else { '░' };

				if self.clip.contains(col, rect.top) {
					self.buffer.set_glyph(col, rect.top, symbol, fg, bg, attrs);
				}
			}
		}
	}

	/// An activity spinner, or a stopped dot.
	fn activity(&mut self, id: NodeId, rect: Rect, fg: TuiColor, bg: TuiColor, attrs: Attributes) {
		let running = self
			.backend
			.props(id)
			.is_none_or(|props| props.running.unwrap_or(true));
		let symbol = if running {
			SPINNER[(self.backend.tick() % SPINNER.len() as u64) as usize]
		} else {
			'·'
		};
		let attrs = if running {
			attrs
		} else {
			attrs.union(Attributes::DIM)
		};

		if self.clip.contains(rect.left, rect.top) {
			self.buffer
				.set_glyph(rect.left, rect.top, symbol, fg, bg, attrs);
		}
	}

	/// An image placeholder: the terminal cannot fetch or decode the source.
	fn image(&mut self, rect: Rect, bg: TuiColor, dim: Attributes) {
		for row in rect.top.max(self.clip.top)..rect.bottom().min(self.clip.bottom()) {
			for col in rect.left.max(self.clip.left)..rect.right().min(self.clip.right()) {
				self.buffer
					.set_glyph(col, row, '▒', TuiColor::PLACEHOLDER, bg, dim);
			}
		}
	}
}

/// The cells a pixel rect covers, snapping each edge to the grid.
fn cell_rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
	let left = (x / CELL_WIDTH).round() as i16;
	let top = (y / CELL_HEIGHT).round() as i16;
	let right = ((x + width) / CELL_WIDTH).round() as i16;
	let bottom = ((y + height) / CELL_HEIGHT).round() as i16;

	Rect::new(
		left,
		top,
		(right - left).max(0) as u16,
		(bottom - top).max(0) as u16,
	)
}

/// The cells one glyph advances.
fn glyph_width(symbol: char) -> i16 {
	unicode_width::UnicodeWidthChar::width(symbol).map_or(0, |width| width as i16)
}

/// The shared packed color type's transparent value, so `unwrap_or` chains
/// stay readable.
fn transparent() -> leptoskit_native::Color {
	leptoskit_native::Color::rgba(0, 0, 0, 0)
}

/// The rows a box covers. Its top snaps to a whole row, as text does, but its
/// height keeps half a row of precision: a box whose height ends mid-row
/// covers only the upper half of its last row, which the painter draws with
/// `▀`, as opencode closes its prompt.
#[derive(Debug, Clone, Copy)]
struct Rows {
	/// The first row.
	top: i16,
	/// The rows covered completely.
	full: i16,
	/// Whether the upper half of the row after the full ones is covered too.
	half: bool,
}

impl Rows {
	fn of(y: f32, height: f32) -> Self {
		let halves = (height / (CELL_HEIGHT / 2.0)).round().max(0.0) as i16;
		Self {
			top: (y / CELL_HEIGHT).round() as i16,
			full: halves / 2,
			half: halves % 2 != 0,
		}
	}

	/// The row just past the full rows.
	const fn full_bottom(self) -> i16 {
		self.top + self.full
	}

	/// The row whose upper half alone the box covers, if any.
	fn half_row(self) -> Option<i16> {
		self.half.then_some(self.full_bottom())
	}

	/// The last row the box touches, or `None` when it has no height.
	fn last(self) -> Option<i16> {
		match (self.full, self.half) {
			(0, false) => None,
			(_, true) => Some(self.full_bottom()),
			(_, false) => Some(self.full_bottom() - 1),
		}
	}
}
