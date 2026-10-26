//! Painting the committed tree into the cell buffer.

use leptoskit_native::FontWeight;
use leptoskit_native::NativeKind;
use leptoskit_native::Progress;
use leptoskit_native::TextAlign;
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
use crate::text::wrap;

/// The braille spinner frames, one per tick.
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// The blocks an indeterminate progress bar sweeps with.
const SWEEP: usize = 3;

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
			NativeKind::Image => self.image(rect, bg, dim),
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
		let bg = TuiColor::from_packed(node.props.background.unwrap_or(transparent()));
		self.buffer.fill(self.clip.intersect(rect), bg);

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
		if node.props.border_width.is_some_and(|width| width > 0.0) {
			let border = TuiColor::from_packed(node.props.border_color.unwrap_or(transparent()));
			self.border(
				rect,
				border,
				dim,
				node.props.corner_radius.unwrap_or(0.0) > 0.0,
			);
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
			if self.clip.contains(col, row) {
				self.buffer.set_glyph(col, row, symbol, fg, bg, attrs);
			}
			col += glyph_width(symbol);
		}
	}

	/// Draws a box border, rounded when asked.
	fn border(&mut self, rect: Rect, color: TuiColor, dim: Attributes, rounded: bool) {
		let attrs = if color == TuiColor::Default {
			dim
		} else {
			Attributes::EMPTY.union(dim)
		};
		let (top_left, top_right, bottom_left, bottom_right) = if rounded {
			('╭', '╮', '╰', '╯')
		} else {
			('┌', '┐', '└', '┘')
		};

		let right = rect.right() - 1;
		let bottom = rect.bottom() - 1;

		for col in rect.left..=right {
			if self.clip.contains(col, rect.top) {
				self.buffer
					.set_glyph(col, rect.top, '─', color, TuiColor::Default, attrs);
			}
			if rect.height > 1 && self.clip.contains(col, bottom) {
				self.buffer
					.set_glyph(col, bottom, '─', color, TuiColor::Default, attrs);
			}
		}

		for row in rect.top..=bottom {
			if self.clip.contains(rect.left, row) {
				self.buffer
					.set_glyph(rect.left, row, '│', color, TuiColor::Default, attrs);
			}
			if rect.width > 1 && self.clip.contains(right, row) {
				self.buffer
					.set_glyph(right, row, '│', color, TuiColor::Default, attrs);
			}
		}

		for (col, row, symbol) in [
			(rect.left, rect.top, top_left),
			(right, rect.top, top_right),
			(rect.left, bottom, bottom_left),
			(right, bottom, bottom_right),
		] {
			if rect.width > 1 && rect.height > 1 && self.clip.contains(col, row) {
				self.buffer
					.set_glyph(col, row, symbol, color, TuiColor::Default, attrs);
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
		let lines = wrap(content, max_cols);
		let rtl = node.props.direction == Some(leptoskit_yoga::Direction::Rtl);

		for (row, line) in lines.iter().enumerate() {
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

			self.write(line, start, line_row, fg, bg, attrs);
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

		if focused {
			let caret = draft.map_or(0, |draft| draft.caret) as i16;
			let col = (rect.left + caret).min(rect.right() - 1);

			if let Some(cell) = self.buffer.cell_mut(
				u16::try_from(col).unwrap_or(0),
				u16::try_from(rect.top).unwrap_or(0),
			) {
				cell.attrs = cell.attrs.union(Attributes::REVERSE);
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
