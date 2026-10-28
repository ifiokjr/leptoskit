//! Widget kinds, properties, and events shared by every platform backend.
//!
//! The kinds are the native widget vocabulary: each maps to a real platform
//! widget (`UIView`/`UITextView` on iOS, an Android view, an `NSView` on macOS,
//! and so on), never to a web view.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use leptoskit_yoga::Direction;
use leptoskit_yoga::Size;

use crate::aria::Aria;
use crate::controls::Progress;
use crate::controls::RadioGroup;
use crate::controls::SliderState;
use crate::vector::Drawing;

/// The kind of native widget an element renders as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
	/// A controlled on/off switch.
	Switch,
	/// A controlled boolean checkbox.
	Checkbox,
	/// A controlled radio option in an explicit group.
	Radio,
	/// A controlled numeric slider.
	Slider,
	/// A horizontal determinate or indeterminate progress bar.
	LinearProgressIndicator,
	/// An animated loading spinner.
	ActivityIndicator,
	/// A secure single-line editable field.
	PasswordInput,
	/// An accessible activation control containing decorative child widgets.
	Pressable,
	/// Filled and stroked vector paths; the drawing comes from the `drawing`
	/// prop.
	Vector,
	/// A platform-defined widget referenced by registered name.
	Custom(&'static str),
}

#[cfg(feature = "serde")]
impl serde::Serialize for NativeKind {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		serializer.serialize_str(self.wire_name())
	}
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
			Self::Switch => "switch",
			Self::Checkbox => "checkbox",
			Self::Radio => "radio",
			Self::Slider => "slider",
			Self::LinearProgressIndicator => "linear_progress_indicator",
			Self::ActivityIndicator => "activity_indicator",
			Self::PasswordInput => "password_input",
			Self::Pressable => "pressable",
			Self::Vector => "vector",
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
		deserializer.deserialize_any(NativeKindVisitor)
	}
}

#[cfg(feature = "serde")]
struct NativeKindVisitor;

#[cfg(feature = "serde")]
impl<'de> serde::de::Visitor<'de> for NativeKindVisitor {
	type Value = NativeKind;

	fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("a wire name, legacy kind name, or custom widget name")
	}

	fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
	where
		E: serde::de::Error,
	{
		match value {
			"view" | "View" => Ok(NativeKind::View),
			"text" | "Text" => Ok(NativeKind::Text),
			"text_run" | "TextRun" => Ok(NativeKind::TextRun),
			"image" | "Image" => Ok(NativeKind::Image),
			"scroll_view" | "ScrollView" => Ok(NativeKind::ScrollView),
			"text_input" | "TextInput" => Ok(NativeKind::TextInput),
			"button" | "Button" => Ok(NativeKind::Button),
			"switch" | "Switch" => Ok(NativeKind::Switch),
			"checkbox" | "Checkbox" => Ok(NativeKind::Checkbox),
			"radio" | "Radio" => Ok(NativeKind::Radio),
			"slider" | "Slider" => Ok(NativeKind::Slider),
			"linear_progress_indicator" | "LinearProgressIndicator" => {
				Ok(NativeKind::LinearProgressIndicator)
			}
			"activity_indicator" | "ActivityIndicator" => Ok(NativeKind::ActivityIndicator),
			"password_input" | "PasswordInput" => Ok(NativeKind::PasswordInput),
			"pressable" | "Pressable" => Ok(NativeKind::Pressable),
			"vector" | "Vector" => Ok(NativeKind::Vector),

			other => {
				Ok(NativeKind::Custom(Box::leak(
					other.to_owned().into_boxed_str(),
				)))
			}
		}
	}

	fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
	where
		M: serde::de::MapAccess<'de>,
	{
		let Some((key, name)) = map.next_entry::<String, String>()? else {
			return Err(serde::de::Error::custom("empty legacy custom kind"));
		};

		if key != "Custom" || map.next_entry::<String, serde::de::IgnoredAny>()?.is_some() {
			return Err(serde::de::Error::custom(
				"expected a single legacy Custom kind",
			));
		}

		Ok(NativeKind::Custom(Box::leak(name.into_boxed_str())))
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
	const CHANNEL_BITS: u32 = 8;
	const GREEN_SHIFT: u32 = Self::CHANNEL_BITS * 2;
	const HEX_RADIX: u32 = 16;
	const RED_SHIFT: u32 = Self::CHANNEL_BITS * 3;
	const RGBA_HEX_DIGITS: usize = 8;
	const RGB_HEX_DIGITS: usize = 6;

	/// Builds a color from red, green, blue, and alpha bytes.
	pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
		Self(
			((red as u32) << Self::RED_SHIFT)
				| ((green as u32) << Self::GREEN_SHIFT)
				| ((blue as u32) << Self::CHANNEL_BITS)
				| (alpha as u32),
		)
	}

	/// Parses a `#rrggbb` or `#rrggbbaa` hex string.
	#[must_use]
	pub fn hex(value: &str) -> Option<Self> {
		let digits = value.strip_prefix('#')?;

		match digits.len() {
			Self::RGB_HEX_DIGITS => {
				let packed = u32::from_str_radix(digits, Self::HEX_RADIX).ok()?;
				Some(Self((packed << Self::CHANNEL_BITS) | 0xff))
			}
			Self::RGBA_HEX_DIGITS => u32::from_str_radix(digits, Self::HEX_RADIX).ok().map(Self),
			_ => None,
		}
	}

	/// The red channel.
	#[must_use]
	pub fn red(self) -> u8 {
		(self.0 >> Self::RED_SHIFT) as u8
	}

	/// The green channel.
	#[must_use]
	pub fn green(self) -> u8 {
		(self.0 >> Self::GREEN_SHIFT) as u8
	}

	/// The blue channel.
	#[must_use]
	pub fn blue(self) -> u8 {
		(self.0 >> Self::CHANNEL_BITS) as u8
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
	/// The controlled selected state of a switch, checkbox, or radio.
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
	pub checked: Option<bool>,
	/// The explicit group of a radio option.
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
	pub radio_group: Option<RadioGroup>,
	/// The validated controlled slider configuration.
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
	pub slider: Option<SliderState>,
	/// The completion state of a progress bar.
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
	pub progress: Option<Progress>,
	/// Whether an activity indicator should animate while visible.
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
	pub running: Option<bool>,
	/// The paths a `Vector` draws. A drawing is one shared pointer, so
	/// cloning and diffing props never copies its paths.
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
	pub drawing: Option<Drawing>,
	/// The foreground color.
	pub color: Option<Color>,
	/// The background color.
	pub background: Option<Color>,
	/// Whether the widget accepts interaction.
	pub enabled: Option<bool>,
	/// An accessibility label read by screen readers; the accessible name
	/// when visible text does not name the widget.
	pub accessibility_label: Option<String>,
	/// The ARIA properties beyond the name: roles, states, values, live
	/// region, and relationships. Serialized as one nested `aria` object and
	/// omitted entirely while unset, so traffic without accessibility stays
	/// byte-for-byte what it was. Boxed, because the recursive tree passes
	/// keep whole elements on their stacks and a deep tree must render inside
	/// a default thread stack.
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
	pub aria: Option<Box<Aria>>,
	/// A stable identifier unique within the tree, for ARIA relationships and
	/// the DOM `id` attribute.
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
	pub id: Option<String>,
	/// The radius of the widget's rounded corners, in layout units.
	///
	/// Content is clipped to the rounded shape, so an image inside a rounded
	/// view gets rounded corners too.
	pub corner_radius: Option<f32>,
	/// The width of the border stroke drawn inside the widget's bounds.
	///
	/// This is purely visual: the layout border lives in the yoga
	/// [`Style`](leptoskit_yoga::Style) and is set separately when the stroke
	/// should take space.
	pub border_width: Option<f32>,
	/// The color of the border stroke.
	pub border_color: Option<Color>,
	/// The widget's opacity, from `0.0` (transparent) to `1.0` (opaque),
	/// applied to the widget and everything inside it.
	pub opacity: Option<f32>,
	/// Whether the widget and its descendants are hidden. Unset means visible.
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
	pub hidden: Option<bool>,
	/// Whether content is clipped to the widget's bounds. Rounded corners
	/// also clip content, even when this is unset or `false`.
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
	pub clip: Option<bool>,
	/// The font size of `Text`, `TextInput`, and `Button`, in layout units.
	///
	/// Unset means [`TextStyle::DEFAULT_FONT_SIZE`].
	pub font_size: Option<f32>,
	/// The font weight of `Text`, `TextInput`, and `Button`.
	pub font_weight: Option<FontWeight>,
	/// The horizontal alignment of text within the widget.
	pub text_align: Option<TextAlign>,
	/// The resolved writing direction for native text. The renderer derives
	/// this from the element's layout direction before committing props.
	#[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
	pub direction: Option<Direction>,
	/// Platform-specific properties, keyed by name.
	pub extra: BTreeMap<String, PropValue>,
}

/// The shared all-unset ARIA group, so `None` needs no allocation. A static
/// rather than the const, because a group with collections cannot promote.
static EMPTY_ARIA: Aria = Aria::EMPTY;

/// Restricts navigation before a raw prop can reach an anchor attribute.
fn is_safe_href(href: &str) -> bool {
	// Browsers remove leading controls and embedded tabs/newlines before
	// interpreting schemes. Reject them rather than validating a different URL.
	if href.is_empty()
		|| href.chars().any(|character| {
			character.is_control() || character.is_whitespace() || character == '\\'
		}) {
		return false;
	}

	// A scheme can only appear before the first path/query/fragment delimiter.
	// Keep the original encoding: percent-encoded colons are ordinary path
	// data.
	let prefix = href
		.split_once(['/', '?', '#'])
		.map_or(href, |(prefix, _)| prefix);
	let Some((scheme, _)) = prefix.split_once(':') else {
		return true;
	};

	["http", "https", "mailto", "tel", "sms"]
		.iter()
		.any(|allowed| scheme.eq_ignore_ascii_case(allowed))
}

impl NativeProps {
	/// The key in [`extra`](Self::extra) that holds a link target.
	pub const HREF: &'static str = "href";

	/// Builds a `Text` value's props with the given content.
	#[must_use]
	pub fn text(content: impl Into<String>) -> Self {
		Self {
			text: Some(content.into()),
			..Self::default()
		}
	}

	/// The link target set by [`Element::href`](crate::Element::href), when
	/// it is a relative reference or uses HTTP(S), mailto, tel, or sms.
	/// Empty targets, whitespace, controls, backslashes, and other schemes
	/// return `None`, including when supplied through [`extra`](Self::extra)
	/// or deserialization.
	#[must_use]
	pub fn href(&self) -> Option<&str> {
		match self.extra.get(Self::HREF) {
			Some(PropValue::Str(href)) if is_safe_href(href) => Some(href),
			_ => None,
		}
	}

	/// The font these props measure and draw text with, resolving unset
	/// fields to their defaults.
	///
	/// The renderer measures with exactly this style and backends draw with
	/// it, so the two always agree.
	#[must_use]
	pub fn text_style(&self) -> TextStyle {
		TextStyle {
			font_size: self.font_size.unwrap_or(TextStyle::DEFAULT_FONT_SIZE),
			font_weight: self.font_weight.unwrap_or_default(),
			text_align: self.text_align.unwrap_or_default(),
			direction: self.direction.unwrap_or_default(),
		}
	}

	/// The ARIA group, allocated only once a property is set; `None` is the
	/// shared all-unset group, so trees without accessibility never allocate.
	#[must_use]
	pub fn aria(&self) -> &Aria {
		self.aria.as_deref().unwrap_or(&EMPTY_ARIA)
	}

	/// The ARIA group for mutation, allocating the shared empty group once.
	pub fn aria_mut(&mut self) -> &mut Aria {
		self.aria.get_or_insert_with(|| Box::new(Aria::EMPTY))
	}

	/// The name assistive technology announces: the accessible label, else
	/// the visible text.
	#[must_use]
	pub fn accessible_name(&self) -> Option<&str> {
		self.accessibility_label.as_deref().or(self.text.as_deref())
	}

	/// Whether this element names itself, for merge and validation rules:
	/// a non-whitespace label or a labelling reference.
	#[must_use]
	pub fn names_itself(&self) -> bool {
		self.accessibility_label
			.as_deref()
			.is_some_and(|label| !label.trim().is_empty())
			|| !self.aria().labelled_by.is_empty()
	}

	/// Whether these props draw the control's own background, border, or
	/// corners, which replace a text field's or button's native chrome.
	#[must_use]
	pub fn is_custom_drawn(&self) -> bool {
		self.background.is_some() || self.border_width.is_some() || self.corner_radius.is_some()
	}
}

/// The weight of a font, from the platform's system font family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(rename_all = "snake_case")
)]
pub enum FontWeight {
	/// The body weight.
	#[default]
	Regular,
	/// Slightly heavier than regular, for emphasis inside body text.
	Medium,
	/// Between medium and bold, for titles and labels.
	Semibold,
	/// The heaviest weight, for headlines and figures.
	Bold,
}

/// The horizontal alignment of text inside its widget.
///
/// `Start` and `End` follow the writing direction, so they flip under
/// right-to-left layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(rename_all = "snake_case")
)]
pub enum TextAlign {
	/// Aligned to the leading edge.
	#[default]
	Start,
	/// Centered.
	Center,
	/// Aligned to the trailing edge.
	End,
}

/// How a text node is set: everything about its props that can change the
/// size a platform needs to draw the text.
///
/// The font is the obvious part. Alignment is here because some text systems
/// reserve different space for it: on macOS, an `NSTextField` label needs a
/// little more width for centered text than for the same text aligned to the
/// start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
	/// The font size, in layout units.
	pub font_size: f32,
	/// The font weight.
	pub font_weight: FontWeight,
	/// The horizontal alignment.
	pub text_align: TextAlign,
	/// The writing direction used for shaping and logical alignment.
	pub direction: Direction,
}

impl TextStyle {
	/// The font size used when a node does not set one.
	pub const DEFAULT_FONT_SIZE: f32 = 14.0;

	/// A start-aligned style with the given size and weight.
	pub const fn new(font_size: f32, font_weight: FontWeight) -> Self {
		Self {
			font_size,
			font_weight,
			text_align: TextAlign::Start,
			direction: Direction::Ltr,
		}
	}
}

impl Default for TextStyle {
	fn default() -> Self {
		Self::new(Self::DEFAULT_FONT_SIZE, FontWeight::Regular)
	}
}

/// The events a native widget can emit.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum NativeEvent {
	/// The widget was activated (tap, click, primary action).
	Press,
	/// A `Button` or `Pressable` started showing its platform's pressed
	/// state: a pointer, finger, or key is holding it down.
	///
	/// Hosts pass it to
	/// [`NativeSurface::press_in`](crate::NativeSurface::press_in),
	/// which applies the control's `active:` utilities until the matching
	/// [`PressOut`](Self::PressOut). Event handlers never receive it.
	PressIn,
	/// A held control was released, cancelled, or left behind by the
	/// pointer, ending the pressed state [`PressIn`](Self::PressIn) started.
	/// When a release activates the control, [`Press`](Self::Press) may
	/// arrive before or after it.
	PressOut,
	/// An editable field's content changed.
	Change {
		/// The new content.
		value: String,
	},
	/// A boolean control proposed a new checked state.
	Toggle {
		/// The proposed selection.
		checked: bool,
	},
	/// A numeric control proposed a new finite value.
	ValueChange {
		/// The proposed value on the configured step grid.
		value: f64,
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
///
/// The renderer calls [`measure`](TextMeasurer::measure) with the node's
/// [`TextStyle`] and re-measures whenever the text or the style changes.
/// Results must be pure functions of the inputs: layout caches them.
pub trait TextMeasurer: Send + Sync {
	/// Measures `text`, set in `style`, under the given available space.
	///
	/// The returned size is the content box the widget needs to draw the text
	/// without clipping it.
	fn measure(
		&self,
		text: &str,
		style: TextStyle,
		available: Size<leptoskit_yoga::AvailableSpace>,
	) -> Size<f32>;

	/// Measures a button title, including platform chrome when requested.
	///
	/// Layout adds caller padding and borders separately. Existing measurers
	/// retain their text behavior; platform implementations may reserve the
	/// native single-line control's intrinsic size without constructing a view.
	fn measure_button(
		&self,
		text: &str,
		style: TextStyle,
		available: Size<leptoskit_yoga::AvailableSpace>,
		_native_chrome: bool,
	) -> Size<f32> {
		self.measure(text, style, available)
	}
}

/// The default text measurer: a fixed 8×16 cell per character at
/// [`TextStyle::DEFAULT_FONT_SIZE`], scaled linearly with the font size and
/// wrapping at word boundaries when width is bounded. Explicit newlines start
/// a new line, including consecutive and trailing empty lines.
///
/// The font weight and the alignment do not change the approximation.
#[derive(Debug, Clone, Copy)]
pub struct ApproxTextMeasurer {
	/// The character cell width at the default font size.
	pub char_width: f32,
	/// The line height at the default font size.
	pub line_height: f32,
}

impl ApproxTextMeasurer {
	/// A measurer with an 8×16 cell at the default font size.
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
	fn measure(
		&self,
		text: &str,
		style: TextStyle,
		available: Size<leptoskit_yoga::AvailableSpace>,
	) -> Size<f32> {
		let scale = style.font_size.max(0.0) / TextStyle::DEFAULT_FONT_SIZE;
		let cell = Size::new(self.char_width * scale, self.line_height * scale);

		let max_width = match available.width {
			leptoskit_yoga::AvailableSpace::Definite(width) => Some(width.max(0.0)),
			_ => None,
		};

		text.split('\n')
			.map(|line| measure_line(line, cell, max_width))
			.fold(Size::new(0.0, 0.0), |total, line| {
				Size::new(total.width.max(line.width), total.height + line.height)
			})
	}
}

/// Keeps indivisible words clamped consistently and reports the widest wrapped
/// line.
fn measure_line(text: &str, cell: Size<f32>, max_width: Option<f32>) -> Size<f32> {
	let Some(max) = max_width else {
		return Size::new(cell.width * text.chars().count() as f32, cell.height);
	};
	let mut lines = 1usize;
	let mut current = 0.0f32;
	let mut widest = 0.0f32;

	for word in text.split(' ') {
		let word_width = (cell.width * word.chars().count() as f32).min(max);
		let space = if current > 0.0 { cell.width } else { 0.0 };

		if current > 0.0 && current + space + word_width > max {
			widest = widest.max(current);
			lines += 1;
			current = word_width;
		} else {
			current += space + word_width;
		}
	}

	Size::new(widest.max(current), cell.height * lines as f32)
}

/// A shared, cloneable text measurer.
pub type SharedTextMeasurer = Arc<dyn TextMeasurer>;
