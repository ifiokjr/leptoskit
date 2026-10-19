//! Widget kinds, properties, and events shared by every platform backend.
//!
//! The kinds are the native widget vocabulary: each maps to a real platform
//! widget (`UIView`/`UITextView` on iOS, an Android view, an `NSView` on macOS,
//! and so on), never to a web view.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use leptoskit_yoga::Size;

/// The kind of native widget an element renders as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum NativeKind {
	/// A plain container view.
	View,
	/// A text label; its content comes from the `text` prop.
	Text,
	/// An unstyled run of text inside a `Text` parent.
	TextRun,
	/// An image; its source comes from the `source` prop.
	Image,
	/// A scrollable container.
	ScrollView,
	/// A single-line editable field.
	TextInput,
	/// A pressable control.
	Button,
	/// A platform-defined widget referenced by registered name.
	Custom(&'static str),
}

impl NativeKind {
	/// The wire name of the kind, for serialization and tooling.
	pub fn wire_name(self) -> &'static str {
		match self {
			Self::View => "view",
			Self::Text => "text",
			Self::TextRun => "text_run",
			Self::Image => "image",
			Self::ScrollView => "scroll_view",
			Self::TextInput => "text_input",
			Self::Button => "button",
			Self::Custom(name) => name,
		}
	}
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for NativeKind {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		struct Visitor;
		impl serde::de::Visitor<'_> for Visitor {
			type Value = NativeKind;

			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				f.write_str("a wire name or a custom widget name")
			}

			fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
			where
				E: serde::de::Error,
			{
				match value {
					"view" => Ok(NativeKind::View),
					"text" => Ok(NativeKind::Text),
					"text_run" => Ok(NativeKind::TextRun),
					"image" => Ok(NativeKind::Image),
					"scroll_view" => Ok(NativeKind::ScrollView),
					"text_input" => Ok(NativeKind::TextInput),
					"button" => Ok(NativeKind::Button),
					other => {
						Ok(NativeKind::Custom(Box::leak(
							other.to_owned().into_boxed_str(),
						)))
					}
				}
			}
		}
		deserializer.deserialize_str(Visitor)
	}
}

impl fmt::Display for NativeKind {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.wire_name())
	}
}

/// An 8-bit-per-channel RGBA color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Color(pub u32);

impl Color {
	/// Builds a color from red, green, blue, and alpha bytes.
	pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
		Self(((red as u32) << 24) | ((green as u32) << 16) | ((blue as u32) << 8) | (alpha as u32))
	}

	/// Parses a `#rrggbb` or `#rrggbbaa` hex string.
	#[must_use]
	pub fn hex(value: &str) -> Option<Self> {
		let digits = value.strip_prefix('#')?;
		match digits.len() {
			6 => {
				let packed = u32::from_str_radix(digits, 16).ok()?;
				Some(Self((packed << 8) | 0xff))
			}
			8 => u32::from_str_radix(digits, 16).ok().map(Self),
			_ => None,
		}
	}

	/// The red channel.
	#[must_use]
	pub fn red(self) -> u8 {
		(self.0 >> 24) as u8
	}

	/// The green channel.
	#[must_use]
	pub fn green(self) -> u8 {
		(self.0 >> 16) as u8
	}

	/// The blue channel.
	#[must_use]
	pub fn blue(self) -> u8 {
		(self.0 >> 8) as u8
	}

	/// The alpha channel.
	#[must_use]
	pub fn alpha(self) -> u8 {
		self.0 as u8
	}
}

/// A platform-neutral property value.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PropValue {
	/// A string.
	Str(String),
	/// A number.
	Number(f64),
	/// A boolean.
	Bool(bool),
	/// A color.
	Color(Color),
}

impl From<&str> for PropValue {
	fn from(value: &str) -> Self {
		Self::Str(value.to_owned())
	}
}

impl From<String> for PropValue {
	fn from(value: String) -> Self {
		Self::Str(value)
	}
}

impl From<f64> for PropValue {
	fn from(value: f64) -> Self {
		Self::Number(value)
	}
}

impl From<bool> for PropValue {
	fn from(value: bool) -> Self {
		Self::Bool(value)
	}
}

impl From<Color> for PropValue {
	fn from(value: Color) -> Self {
		Self::Color(value)
	}
}

/// The common properties every backend understands.
///
/// Layout lives in the yoga [`Style`](leptoskit_yoga::Style); these are the
/// widget's own values. Anything platform-specific rides in `extra`.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(default)
)]
pub struct NativeProps {
	/// The text content of `Text`, `TextRun`, `TextInput`, and `Button`.
	pub text: Option<String>,
	/// The image or content source URI.
	pub source: Option<String>,
	/// The placeholder for `TextInput`.
	pub placeholder: Option<String>,
	/// The current value of `TextInput`.
	pub value: Option<String>,
	/// The foreground color.
	pub color: Option<Color>,
	/// The background color.
	pub background: Option<Color>,
	/// Whether the widget accepts interaction.
	pub enabled: Option<bool>,
	/// An accessibility label read by screen readers.
	pub accessibility_label: Option<String>,
	/// Platform-specific properties, keyed by name.
	pub extra: BTreeMap<String, PropValue>,
}

impl NativeProps {
	/// Builds a `Text` value's props with the given content.
	#[must_use]
	pub fn text(content: impl Into<String>) -> Self {
		Self {
			text: Some(content.into()),
			..Self::default()
		}
	}
}

/// The events a native widget can emit.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum NativeEvent {
	/// The widget was activated (tap, click, primary action).
	Press,
	/// An editable field's content changed.
	Change {
		/// The new content.
		value: String,
	},
	/// An editable field submitted (enter, done).
	Submit {
		/// The submitted content.
		value: String,
	},
	/// The widget gained focus.
	Focus,
	/// The widget lost focus.
	Blur,
	/// A scrollable view scrolled.
	Scroll {
		/// The horizontal scroll offset.
		offset_x: f32,
		/// The vertical scroll offset.
		offset_y: f32,
	},
}

/// Whether an event was handled during dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventOutcome {
	/// A handler consumed the event; bubbling stops.
	Handled,
	/// No handler consumed the event.
	Unhandled,
}

/// A handler for one event on one element.
pub type EventHandler = Arc<dyn Fn(&NativeEvent) -> EventOutcome + Send + Sync>;

/// Measures intrinsic text size for layout.
///
/// Backends with real platform text systems supply their own implementation;
/// the default is the fixed-cell approximation below.
pub trait TextMeasurer: Send + Sync {
	/// Measures `text` under the given available space.
	fn measure(&self, text: &str, available: Size<leptoskit_yoga::AvailableSpace>) -> Size<f32>;
}

/// The default text measurer: a fixed 8×16 cell per character, wrapping at
/// word boundaries when width is bounded.
#[derive(Debug, Clone, Copy)]
pub struct ApproxTextMeasurer {
	/// The character cell width.
	pub char_width: f32,
	/// The line height.
	pub line_height: f32,
}

impl ApproxTextMeasurer {
	/// A measurer with an 8×16 cell.
	pub const fn new() -> Self {
		Self {
			char_width: 8.0,
			line_height: 16.0,
		}
	}
}

impl Default for ApproxTextMeasurer {
	fn default() -> Self {
		Self::new()
	}
}

impl TextMeasurer for ApproxTextMeasurer {
	fn measure(&self, text: &str, available: Size<leptoskit_yoga::AvailableSpace>) -> Size<f32> {
		let max_width = match available.width {
			leptoskit_yoga::AvailableSpace::Definite(width) => Some(width.max(0.0)),
			_ => None,
		};
		let mut lines = 1usize;
		let mut current = 0.0f32;
		if let Some(max) = max_width {
			for word in text.split(' ') {
				let word_width = self.char_width * word.chars().count() as f32;
				let space = if current > 0.0 { self.char_width } else { 0.0 };
				if current + space + word_width > max && current > 0.0 {
					lines += 1;
					current = word_width.min(max);
				} else {
					current += space + word_width;
				}
			}
			Size::new(current.max(0.0), self.line_height * lines as f32)
		} else {
			let width = self.char_width * text.chars().count() as f32;
			Size::new(width, self.line_height)
		}
	}
}

/// A shared, cloneable text measurer.
pub type SharedTextMeasurer = Arc<dyn TextMeasurer>;
