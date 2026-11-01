//! The binary render transaction the generated iOS and Android apps read.
//!
//! Decoding the JSON transaction cost the platforms more than some of the
//! work it describes: Swift's `JSONDecoder` took 30 ms for the 6,000
//! mutations that create 1,000 cards, and Android parsed every batch into a
//! tree of `JSONObject`s, then looked each property up by name. This form is
//! read with a cursor and fixed-width loads instead. It carries exactly what
//! the JSON transaction does: properties the renderers apply on every view
//! are typed fields, and any other property that is set travels in the
//! JSON form, inside the record.
//!
//! Little endian throughout:
//!
//! ```text
//! transaction = "LKB1" root:i64 (-1 for none) count:u32 mutation{count}
//! mutation    = 1 id:u32 kind:str8 props          create
//!             | 2 id:u32 props                    update props
//!             | 3 id:u32 x:f32 y:f32 w:f32 h:f32  update layout
//!             | 4 parent:u32 child:u32 index:u32  insert child
//!             | 5 parent:u32 child:u32            remove child
//!             | 6 id:u32                          delete
//! props       = present:u32, then each present field in bit order:
//!                0 text str             1 source str
//!                2 placeholder str      3 value str
//!                4 accessibility_label str
//!                5 color u32            6 background u32
//!                7 border_color u32     8 enabled u8
//!                9 hidden u8           10 clip u8
//!               11 corner_radius f32   12 border_width f32
//!               13 opacity f32         14 font_size f32
//!               15 font_weight u8 (0 regular, 1 medium, 2 semibold, 3 bold)
//!               16 text_align u8 (0 start, 1 center, 2 end)
//!               31 rest str: every other set property, as the JSON object
//!                  the JSON transaction sends for props
//! str8        = length:u8 utf8
//! str         = length:u32 utf8
//! ```

use std::io;

use leptoskit_native::FontWeight;
use leptoskit_native::Mutation;
use leptoskit_native::NativeProps;
use leptoskit_native::TextAlign;

/// The first four bytes of every binary transaction.
pub(super) const MAGIC: &[u8; 4] = b"LKB1";

const CREATE: u8 = 1;
const UPDATE_PROPS: u8 = 2;
const UPDATE_LAYOUT: u8 = 3;
const INSERT_CHILD: u8 = 4;
const REMOVE_CHILD: u8 = 5;
const DELETE: u8 = 6;

const TEXT: u32 = 1 << 0;
const SOURCE: u32 = 1 << 1;
const PLACEHOLDER: u32 = 1 << 2;
const VALUE: u32 = 1 << 3;
const ACCESSIBILITY_LABEL: u32 = 1 << 4;
const COLOR: u32 = 1 << 5;
const BACKGROUND: u32 = 1 << 6;
const BORDER_COLOR: u32 = 1 << 7;

const ENABLED: u32 = 1 << 8;
const HIDDEN: u32 = 1 << 9;
const CLIP: u32 = 1 << 10;
const CORNER_RADIUS: u32 = 1 << 11;
const BORDER_WIDTH: u32 = 1 << 12;
const OPACITY: u32 = 1 << 13;
const FONT_SIZE: u32 = 1 << 14;
const FONT_WEIGHT: u32 = 1 << 15;
const TEXT_ALIGN: u32 = 1 << 16;
const REST: u32 = 1 << 31;

/// Encodes one commit's mutations, failing once the transaction would pass
/// `limit` bytes.
pub(super) fn encode(
	root: Option<usize>,
	mutations: &[Mutation],
	limit: usize,
) -> io::Result<Vec<u8>> {
	let mut writer = Writer {
		bytes: Vec::with_capacity(16 + mutations.len() * 24),
		limit,
	};
	writer.put(MAGIC)?;

	let root = match root {
		Some(root) => i64::try_from(root).map_err(|_| too_large("root id"))?,
		None => -1,
	};

	writer.put(&root.to_le_bytes())?;
	writer.u32(u32::try_from(mutations.len()).map_err(|_| too_large("mutation count"))?)?;

	for mutation in mutations {
		match mutation {
			Mutation::Create { id, kind, props } => {
				writer.put(&[CREATE])?;
				writer.id(id.index())?;
				let name = kind.wire_name();
				writer.put(&[u8::try_from(name.len()).map_err(|_| too_large("kind name"))?])?;
				writer.put(name.as_bytes())?;
				writer.props(props)?;
			}
			Mutation::UpdateProps { id, props } => {
				writer.put(&[UPDATE_PROPS])?;
				writer.id(id.index())?;
				writer.props(props)?;
			}
			Mutation::UpdateLayout {
				id,
				x,
				y,
				width,
				height,
			} => {
				writer.put(&[UPDATE_LAYOUT])?;
				writer.id(id.index())?;

				for value in [x, y, width, height] {
					writer.put(&value.to_le_bytes())?;
				}
			}
			Mutation::InsertChild {
				parent,
				child,
				index,
			} => {
				writer.put(&[INSERT_CHILD])?;
				writer.id(parent.index())?;
				writer.id(child.index())?;
				writer.id(*index)?;
			}
			Mutation::RemoveChild { parent, child } => {
				writer.put(&[REMOVE_CHILD])?;
				writer.id(parent.index())?;
				writer.id(child.index())?;
			}
			Mutation::Delete { id } => {
				writer.put(&[DELETE])?;
				writer.id(id.index())?;
			}
		}
	}

	Ok(writer.bytes)
}

fn too_large(what: &str) -> io::Error {
	io::Error::other(format!("render transaction {what} out of range"))
}

struct Writer {
	bytes: Vec<u8>,
	limit: usize,
}

impl Writer {
	fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
		if bytes.len() > self.limit - self.bytes.len() {
			return Err(io::Error::other("render transaction byte limit exceeded"));
		}

		self.bytes.extend_from_slice(bytes);
		Ok(())
	}

	fn u32(&mut self, value: u32) -> io::Result<()> {
		self.put(&value.to_le_bytes())
	}

	fn id(&mut self, value: usize) -> io::Result<()> {
		self.u32(u32::try_from(value).map_err(|_| too_large("id"))?)
	}

	fn string(&mut self, value: &str) -> io::Result<()> {
		self.u32(u32::try_from(value.len()).map_err(|_| too_large("string"))?)?;
		self.put(value.as_bytes())
	}

	fn props(&mut self, props: &NativeProps) -> io::Result<()> {
		// Every property sent typed is cleared here; whatever remains set
		// travels in the JSON form, so a property the typed fields do not
		// name is never lost.
		let mut rest = props.clone();
		let text = rest.text.take();
		let source = rest.source.take();
		let placeholder = rest.placeholder.take();
		let value = rest.value.take();
		let accessibility_label = rest.accessibility_label.take();
		let color = rest.color.take();
		let background = rest.background.take();

		let border_color = rest.border_color.take();
		let enabled = rest.enabled.take();
		let hidden = rest.hidden.take();
		let clip = rest.clip.take();
		let corner_radius = rest.corner_radius.take();
		let border_width = rest.border_width.take();
		let opacity = rest.opacity.take();
		let font_size = rest.font_size.take();
		let font_weight = rest.font_weight.take();
		let text_align = rest.text_align.take();
		let rest = (rest != NativeProps::default())
			.then(|| serde_json::to_string(&rest))
			.transpose()
			.map_err(io::Error::other)?;

		let mut present = 0;
		for (bit, set) in [
			(TEXT, text.is_some()),
			(SOURCE, source.is_some()),
			(PLACEHOLDER, placeholder.is_some()),
			(VALUE, value.is_some()),
			(ACCESSIBILITY_LABEL, accessibility_label.is_some()),
			(COLOR, color.is_some()),
			(BACKGROUND, background.is_some()),
			(BORDER_COLOR, border_color.is_some()),
			(ENABLED, enabled.is_some()),
			(HIDDEN, hidden.is_some()),
			(CLIP, clip.is_some()),
			(CORNER_RADIUS, corner_radius.is_some()),
			(BORDER_WIDTH, border_width.is_some()),
			(OPACITY, opacity.is_some()),
			(FONT_SIZE, font_size.is_some()),
			(FONT_WEIGHT, font_weight.is_some()),
			(TEXT_ALIGN, text_align.is_some()),
			(REST, rest.is_some()),
		] {
			if set {
				present |= bit;
			}
		}
		self.u32(present)?;

		for value in [&text, &source, &placeholder, &value, &accessibility_label]
			.into_iter()
			.flatten()
		{
			self.string(value)?;
		}

		for value in [color, background, border_color].into_iter().flatten() {
			self.u32(value.0)?;
		}

		for value in [enabled, hidden, clip].into_iter().flatten() {
			self.put(&[u8::from(value)])?;
		}

		for value in [corner_radius, border_width, opacity, font_size]
			.into_iter()
			.flatten()
		{
			self.put(&value.to_le_bytes())?;
		}

		if let Some(weight) = font_weight {
			self.put(&[match weight {
				FontWeight::Regular => 0,
				FontWeight::Medium => 1,
				FontWeight::Semibold => 2,
				FontWeight::Bold => 3,
			}])?;
		}

		if let Some(align) = text_align {
			self.put(&[match align {
				TextAlign::Start => 0,
				TextAlign::Center => 1,
				TextAlign::End => 2,
			}])?;
		}

		if let Some(rest) = rest {
			self.string(&rest)?;
		}

		Ok(())
	}
}

#[cfg(test)]
mod tests;
