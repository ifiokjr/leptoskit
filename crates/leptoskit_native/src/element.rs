//! The declarative element tree apps build each frame.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;

use leptoskit_yoga::Style;

use crate::aria::Aria;
use crate::aria::Current;
use crate::aria::HasPopup;
use crate::aria::Invalid;
use crate::aria::Live;
use crate::aria::Orientation;
use crate::aria::Role;
use crate::aria::Tristate;
use crate::controls::Progress;
use crate::controls::RadioGroup;
use crate::controls::SliderState;
use crate::props::Color;
use crate::props::EventHandler;
use crate::props::FontWeight;
use crate::props::NativeEvent;
use crate::props::NativeKind;
use crate::props::NativeProps;
use crate::props::PropValue;
use crate::props::TextAlign;
use crate::tailwind::ClassError;
use crate::tailwind::StyleContext;
use crate::tailwind::Tailwind;
use crate::vector::Drawing;
use crate::vector::VectorAlignment;
use crate::vector::VectorPath;
use crate::vector::VectorScaling;
use crate::vector::ViewBox;

/// One node of a declarative interface: a widget kind, its style, its props,
/// its children, and its event handlers.
///
/// Elements are cheap values that describe an interface; the
/// [`NativeSurface`](crate::NativeSurface) diffs them against the committed
/// tree and turns the difference into native mutations. This mirrors how a
/// leptos component's view is a description that the renderer reconciles.
#[derive(Debug, Clone)]
pub struct Element {
	/// The widget kind.
	pub kind: NativeKind,
	/// The layout style.
	pub style: Style,
	/// The widget properties.
	pub props: NativeProps,
	/// The children, in order.
	pub children: Vec<Element>,
	classes: Tailwind,
	handlers: Vec<HandlerEntry>,
}

#[derive(Clone)]
pub(crate) struct HandlerEntry {
	pub(crate) event: NativeEventSlot,
	pub(crate) handler: EventHandler,
}

/// Which events an element listens for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeEventSlot {
	/// [`NativeEvent::Press`].
	Press,
	/// [`NativeEvent::Change`].
	Change,
	/// [`NativeEvent::Submit`].
	Submit,
	/// [`NativeEvent::Focus`].
	Focus,
	/// [`NativeEvent::Blur`].
	Blur,
	/// [`NativeEvent::Toggle`].
	Toggle,
	/// [`NativeEvent::ValueChange`].
	ValueChange,
	/// [`NativeEvent::Scroll`].
	Scroll,
	/// [`NativeEvent::Key`].
	Key,
}

impl NativeEventSlot {
	/// Whether a concrete event belongs to this slot.
	pub fn matches(self, event: &NativeEvent) -> bool {
		matches!(
			(self, event),
			(Self::Press, NativeEvent::Press)
				| (Self::Change, NativeEvent::Change { .. })
				| (Self::Submit, NativeEvent::Submit { .. })
				| (Self::Focus, NativeEvent::Focus)
				| (Self::Blur, NativeEvent::Blur)
				| (Self::Scroll, NativeEvent::Scroll { .. })
				| (Self::Toggle, NativeEvent::Toggle { .. })
				| (Self::ValueChange, NativeEvent::ValueChange { .. })
				| (Self::Key, NativeEvent::Key(_))
		)
	}
}

impl std::fmt::Debug for HandlerEntry {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("HandlerEntry")
			.field("event", &self.event)
			.finish_non_exhaustive()
	}
}

/// The share of its opacity a held control without `active:` utilities
/// keeps. Strong enough to read through a quick tap, light enough not to
/// flash; the web adapter emits the same factor as an `opacity()` filter.
pub(crate) const DEFAULT_PRESSED_OPACITY: f32 = 0.7;

/// Whether a kind has a pressed state for `active:` utilities to style.
pub(crate) fn is_pressable(kind: NativeKind) -> bool {
	matches!(kind, NativeKind::Button | NativeKind::Pressable)
}

impl Element {
	/// Checks structural control and accessibility contracts before any
	/// backend mutations occur.
	///
	/// Radio names are scoped to this complete tree. At most one option may be
	/// checked in each group; the application owns that selection. Ids are
	/// unique within the tree, ARIA relationships reference ids that exist,
	/// and pressables and text inputs carry an accessible name.
	pub fn validate(&self) -> Result<(), crate::controls::ControlTreeError> {
		use crate::controls::ControlTreeError;

		let ids = self.collect_ids()?;
		let mut pending = vec![(self, None)];
		let mut selected_groups = BTreeSet::new();

		while let Some((element, button_ancestor)) = pending.pop() {
			#[cfg(not(feature = "rive"))]
			if element.kind == NativeKind::Rive {
				return Err(ControlTreeError::RiveNotEnabled);
			}
			if let Some(container) = button_ancestor
				&& (matches!(
					element.kind,
					NativeKind::Button
						| NativeKind::Pressable
						| NativeKind::ScrollView
						| NativeKind::TextInput
						| NativeKind::PasswordInput
						| NativeKind::Switch
						| NativeKind::Checkbox
						| NativeKind::Radio
						| NativeKind::Slider
				) || element.style.overflow == leptoskit_yoga::Overflow::Scroll
					|| element.classes.may_scroll()
					|| (container == NativeKind::Pressable
						&& (!element.handlers.is_empty()
							|| matches!(element.kind, NativeKind::Custom(_)))))
			{
				return Err(ControlTreeError::InteractiveDescendant {
					container,
					descendant: element.kind,
				});
			}

			if element.kind == NativeKind::Pressable && !element.props.names_itself() {
				return Err(ControlTreeError::MissingPressableLabel);
			}

			if element.kind != NativeKind::Rive && element.props.rive.is_some() {
				return Err(ControlTreeError::InvalidRive);
			}

			if element.kind == NativeKind::Rive
				&& (element.props.rive.is_none()
					|| element.props.source.is_some()
					|| (element
						.props
						.accessibility_label
						.as_deref()
						.is_none_or(|label| label.trim().is_empty())
						&& element.props.aria().hidden != Some(true)))
			{
				return Err(ControlTreeError::InvalidRive);
			}

			if matches!(
				element.kind,
				NativeKind::Image
					| NativeKind::Rive
					| NativeKind::TextInput
					| NativeKind::PasswordInput
					| NativeKind::Switch
					| NativeKind::Checkbox
					| NativeKind::Radio
					| NativeKind::Slider
					| NativeKind::LinearProgressIndicator
					| NativeKind::ActivityIndicator
					| NativeKind::Vector
			) && !element.children.is_empty()
			{
				return Err(ControlTreeError::Children { kind: element.kind });
			}

			if matches!(
				element.kind,
				NativeKind::TextInput | NativeKind::PasswordInput
			) && !element.props.names_itself()
				&& element
					.props
					.placeholder
					.as_deref()
					.is_none_or(|placeholder| placeholder.trim().is_empty())
			{
				return Err(ControlTreeError::MissingInputLabel { kind: element.kind });
			}

			element.validate_aria(&ids)?;

			if element.kind == NativeKind::Radio {
				let group = element
					.props
					.radio_group
					.as_ref()
					.ok_or(ControlTreeError::MissingRadioGroup)?;

				if element.props.checked == Some(true) && !selected_groups.insert(group.name()) {
					return Err(ControlTreeError::MultipleSelectedRadio {
						group: group.name().to_owned(),
					});
				}
			}

			let button_ancestor =
				if matches!(element.kind, NativeKind::Button | NativeKind::Pressable) {
					Some(element.kind)
				} else {
					button_ancestor
				};
			pending.extend(
				element
					.children
					.iter()
					.map(|child| (child, button_ancestor)),
			);
		}

		Ok(())
	}

	/// Every id in the tree, rejecting duplicates.
	fn collect_ids(&self) -> Result<BTreeSet<&str>, crate::controls::ControlTreeError> {
		let mut ids = BTreeSet::new();
		let mut pending = vec![self];

		while let Some(element) = pending.pop() {
			if let Some(id) = element.props.id.as_deref()
				&& !ids.insert(id)
			{
				return Err(crate::controls::ControlTreeError::DuplicateId { id: id.to_owned() });
			}

			pending.extend(&element.children);
		}

		Ok(ids)
	}

	/// Checks one element's ARIA properties against the spec rules that do not
	/// depend on the platform.
	fn validate_aria(&self, ids: &BTreeSet<&str>) -> Result<(), crate::controls::ControlTreeError> {
		use crate::controls::ControlTreeError;

		let aria = self.props.aria();

		if aria.role.is_some_and(Role::is_presentational) && self.props.names_itself() {
			return Err(ControlTreeError::LabelOnPresentation);
		}

		if aria.level == Some(0) {
			return Err(ControlTreeError::InvalidAriaLevel);
		}

		for (attribute, references) in [
			("aria-labelledby", &aria.labelled_by),
			("aria-describedby", &aria.described_by),
			("aria-controls", &aria.controls),
		] {
			if let Some(id) = references.iter().find(|id| !ids.contains(id.as_str())) {
				return Err(ControlTreeError::UnknownIdReference {
					attribute,
					id: id.clone(),
				});
			}
		}

		if let Some(name) = aria
			.extra
			.keys()
			.find(|name| !crate::aria::accepts_attribute(name))
		{
			return Err(ControlTreeError::InvalidAriaAttribute { name: name.clone() });
		}

		Ok(())
	}

	/// Creates an element of the given kind with default style and props.
	pub fn new(kind: NativeKind) -> Self {
		Self {
			kind,
			style: Style::default(),
			props: NativeProps::default(),
			children: Vec::new(),
			classes: Tailwind::default(),
			handlers: Vec::new(),
		}
	}

	/// A `View` element.
	pub fn view() -> Self {
		Self::new(NativeKind::View)
	}

	/// A `Text` element with the given content.
	pub fn text(content: impl Into<String>) -> Self {
		let mut element = Self::new(NativeKind::Text);
		element.props.text = Some(content.into());
		element
	}

	/// An `Image` element with the given source.
	pub fn image(source: impl Into<String>) -> Self {
		let mut element = Self::new(NativeKind::Image);
		element.props.source = Some(source.into());
		element
	}

	/// A `Vector` element drawing paths in `view_box` coordinates, scaled to
	/// fit its frame. Add paths with [`Self::path`].
	pub fn vector(view_box: ViewBox) -> Self {
		let mut element = Self::new(NativeKind::Vector);
		element.props.drawing = Some(Drawing::new(view_box));
		element
	}

	/// A Rive scene owned and animated by the native host. Give it an explicit
	/// size and an accessible label, or mark it decorative with `aria_hidden`.
	/// Playback is initially paused; use `running(true)` to request animation.
	pub fn rive(playback: leptoskit_rive::Playback) -> Self {
		let mut element = Self::new(NativeKind::Rive);
		element.props.rive = Some(Box::new(playback));
		element
	}

	/// Sets the content of a `Rive` view tag. Parsed players are retained when
	/// the asset, artboard, and scene are unchanged.
	#[must_use]
	pub fn rive_playback(mut self, playback: leptoskit_rive::Playback) -> Self {
		self.props.rive = Some(Box::new(playback));
		self
	}

	/// A `ScrollView` element; its children scroll when they overflow it.
	pub fn scroll_view() -> Self {
		Self::new(NativeKind::ScrollView)
	}

	/// A `TextInput` element.
	pub fn text_input() -> Self {
		Self::new(NativeKind::TextInput)
	}

	/// A `Button` element with the given label.
	pub fn button(label: impl Into<String>) -> Self {
		let mut element = Self::new(NativeKind::Button);
		element.style.justify_content = leptoskit_yoga::JustifyContent::Center;
		element.props.text = Some(label.into());
		element
	}

	/// Appends built-in Tailwind utilities to this component.
	///
	/// Classes override the typed layout and visual properties, regardless of
	/// builder call order. Responsive rules are retained until rendering, so
	/// viewport changes do not require rebuilding the screen. `active:` rules
	/// style a `Button` or `Pressable` while it is held down; other kinds have
	/// no pressed state to style.
	///
	/// # Panics
	///
	/// Panics with a diagnostic for an invalid or unsupported class. Use
	/// [`Self::try_class`] when classes come from dynamic input.
	#[must_use]
	pub fn class(self, classes: impl AsRef<str>) -> Self {
		self.try_class(classes)
			.unwrap_or_else(|error| panic!("invalid leptoskit class: {error}"))
	}

	/// Appends utilities, returning a diagnostic before any rendering occurs.
	pub fn try_class(mut self, classes: impl AsRef<str>) -> Result<Self, ClassError> {
		// Most elements get one class string; joining strings without
		// `format!` matters inside the native guest's interpreter, where
		// formatting cost more than the cached parse itself.
		let existing = self.classes.classes();
		let parsed = if existing.is_empty() {
			Tailwind::parse(classes.as_ref())?
		} else {
			let mut combined = String::with_capacity(existing.len() + 1 + classes.as_ref().len());
			combined.push_str(existing);
			combined.push(' ');
			combined.push_str(classes.as_ref());
			Tailwind::parse(&combined)?
		};

		// On the web, `:active` also matches containers while the pointer is
		// held anywhere inside them, which native platforms cannot report.
		// Pressed styles stay on the controls themselves so both agree.
		if !is_pressable(self.kind)
			&& let Some(class) = parsed.active_class()
		{
			return Err(ClassError {
				class: class.to_owned(),
				reason: format!(
					"`active:` styles a pressed Button or Pressable, and {} has no pressed state",
					self.kind
				),
			});
		}

		self.classes = parsed;
		Ok(self)
	}

	/// The parsed utility program shared by native and web rendering.
	pub fn classes(&self) -> &Tailwind {
		&self.classes
	}

	/// Resolves this tree for an environment, including inherited text styles
	/// and accessibility relationships.
	///
	/// The source tree remains unchanged. The returned snapshot retains no
	/// classes: it is ready for backend reconciliation or serialization.
	/// Names referenced through `aria_labelled_by` and `aria_described_by`
	/// are folded into `label` and `description`, radio options learn their
	/// position in their group, and live regions carry the text to announce,
	/// so native backends never resolve relationships themselves.
	///
	/// A held control, one resolved with [`StyleContext::active`], takes its
	/// `active:` utilities. Without any, a `Button` drawing its own
	/// background, border, or corners and every `Pressable` dim to
	/// seventy percent of their opacity instead; a button keeping its native
	/// chrome keeps the platform's own pressed highlight.
	#[must_use]
	pub fn resolved(&self, context: &StyleContext) -> Self {
		self.resolved_holding(context, None)
	}

	/// Resolves with the control at child-index path `held` pressed, the
	/// empty path naming this element.
	pub(crate) fn resolved_holding(&self, context: &StyleContext, held: Option<&[usize]>) -> Self {
		let (mut element, uses_aria) =
			self.resolve_inherited(context, &NativeProps::default(), held);
		// Nothing resolves relationships, set positions, or live text in a
		// tree that declares none, and the walk would dominate commits.
		if uses_aria {
			element.resolve_accessibility();
		}
		element
	}

	fn resolve_inherited(
		&self,
		context: &StyleContext,
		parent: &NativeProps,
		held: Option<&[usize]>,
	) -> (Self, bool) {
		// Resolve each node once. Cloning its descendants here would repeat
		// the entire subtree clone at every level of a nested component tree.
		let mut element = Self {
			kind: self.kind,
			style: self.style.clone(),
			props: self.props.clone(),
			children: Vec::new(),
			classes: Tailwind::default(),
			handlers: self.handlers.clone(),
		};
		element.props.color = element.props.color.or(parent.color);
		element.props.font_size = element.props.font_size.or(parent.font_size);
		element.props.font_weight = element.props.font_weight.or(parent.font_weight);
		element.props.italic = element.props.italic.or(parent.italic);
		element.props.text_align = element.props.text_align.or(parent.text_align);

		if parent.enabled == Some(false) {
			element.props.enabled = Some(false);
		}

		if parent.aria().hidden == Some(true) {
			element.props.aria_mut().hidden = Some(true);
		}

		// Disabled controls cannot be pressed, even if a hold began before
		// the application disabled them.
		let pressed = is_pressable(self.kind)
			&& element.props.enabled != Some(false)
			&& (context.active || held.is_some_and(<[usize]>::is_empty));
		self.classes.apply(
			&mut element.style,
			&mut element.props,
			&StyleContext {
				active: pressed,
				..*context
			},
		);
		element.props.direction = Some(element.style.direction);

		// Native backends paint fixed colors; `currentColor` becomes the text
		// color this element inherited or set, else CSS's initial black. The
		// web keeps the keyword and lets CSS resolve it.
		if let Some(drawing) = &mut element.props.drawing {
			drawing.resolve_current_color(element.props.color.unwrap_or(Color::rgba(0, 0, 0, 255)));
		}

		if element.style.overflow == leptoskit_yoga::Overflow::Scroll
			&& element.kind == NativeKind::View
		{
			element.kind = NativeKind::ScrollView;
		}

		if element.kind == NativeKind::ScrollView
			&& (element.style.overflow == leptoskit_yoga::Overflow::Hidden
				|| element.props.clip == Some(false))
		{
			element.kind = NativeKind::View;
		}

		element.props.hidden = (element.style.display == leptoskit_yoga::Display::None)
			.then_some(true)
			.or(element.props.hidden);

		if parent.hidden == Some(true) {
			element.props.hidden = Some(true);
		}

		element.props.clip = (element.style.overflow != leptoskit_yoga::Overflow::Visible
			|| element.kind == NativeKind::ScrollView
			|| element
				.props
				.corner_radius
				.is_some_and(|radius| radius > 0.0))
		.then_some(true)
		.or(element.props.clip);

		if pressed
			&& self.classes.active_class().is_none()
			&& (self.kind == NativeKind::Pressable || element.props.is_custom_drawn())
		{
			element.props.opacity =
				Some(element.props.opacity.unwrap_or(1.0) * DEFAULT_PRESSED_OPACITY);
		}

		let uses_aria = element.props.id.is_some()
			|| !element.props.aria().labelled_by.is_empty()
			|| !element.props.aria().described_by.is_empty()
			|| element.kind == NativeKind::Radio
			|| element.props.aria().live.is_some()
			|| element
				.props
				.aria()
				.role
				.is_some_and(|role| role.implicit_live().is_some());
		let mut children_uses_aria = false;
		element.children = self
			.children
			.iter()
			.enumerate()
			.map(|(index, child)| {
				let held = match held {
					Some([first, rest @ ..]) if *first == index => Some(rest),
					_ => None,
				};
				let (child, uses) = child.resolve_inherited(context, &element.props, held);
				children_uses_aria |= uses;
				child
			})
			.collect();
		(element, uses_aria || children_uses_aria)
	}

	/// Folds relationships, radio set positions, and live-region text into
	/// concrete fields, since native accessibility APIs have no id references.
	fn resolve_accessibility(&mut self) {
		let mut names = BTreeMap::new();
		self.collect_names(&mut names);
		let mut set_sizes = BTreeMap::new();
		self.count_radios(&mut set_sizes);
		let mut positions = BTreeMap::new();
		self.fill_accessibility(&names, &set_sizes, &mut positions);
	}

	/// The accessible text of each element with an id.
	fn collect_names(&self, names: &mut BTreeMap<String, String>) {
		if let Some(id) = &self.props.id {
			names.insert(id.clone(), self.text_content());
		}

		for child in &self.children {
			child.collect_names(names);
		}
	}

	/// The number of options in each radio group.
	fn count_radios(&self, sizes: &mut BTreeMap<String, u32>) {
		if self.kind == NativeKind::Radio
			&& let Some(group) = &self.props.radio_group
		{
			*sizes.entry(group.name().to_owned()).or_insert(0) += 1;
		}

		for child in &self.children {
			child.count_radios(sizes);
		}
	}

	fn fill_accessibility(
		&mut self,
		names: &BTreeMap<String, String>,
		set_sizes: &BTreeMap<String, u32>,
		positions: &mut BTreeMap<String, u32>,
	) {
		let referenced = |ids: &[String]| {
			let joined = ids
				.iter()
				.filter_map(|id| names.get(id))
				.filter(|name| !name.is_empty())
				.cloned()
				.collect::<Vec<_>>()
				.join(" ");
			(!joined.is_empty()).then_some(joined)
		};

		if let Some(label) = referenced(&self.props.aria().labelled_by) {
			self.props.accessibility_label = Some(label);
		}

		if let Some(description) = referenced(&self.props.aria().described_by) {
			self.props.aria_mut().description = Some(description);
		}

		if self.kind == NativeKind::Radio
			&& let Some(group) = self.props.radio_group.as_ref().map(RadioGroup::name)
		{
			let position = positions.entry(group.to_owned()).or_insert(0);
			*position += 1;
			let set_size = set_sizes.get(group).copied();
			let aria = self.props.aria_mut();
			aria.pos_in_set = aria.pos_in_set.or(Some(*position));
			aria.set_size = aria.set_size.or(set_size);
		}

		if self.props.aria().effective_live(self.kind) != Live::Off
			&& !self.props.aria().is_hidden()
			&& self.props.hidden != Some(true)
		{
			let content = self.text_content();
			self.props.aria_mut().live_text = Some(content);
		}

		for child in &mut self.children {
			child.fill_accessibility(names, set_sizes, positions);
		}
	}

	/// The text assistive technology reads for this subtree: the accessible
	/// label when set, else the visible text of every descendant that is not
	/// hidden, in document order.
	#[must_use]
	pub fn text_content(&self) -> String {
		if let Some(label) = self
			.props
			.accessibility_label
			.as_deref()
			.filter(|label| !label.trim().is_empty())
		{
			return label.trim().to_owned();
		}

		let mut parts = Vec::new();

		if let Some(text) = self
			.props
			.text
			.as_deref()
			.filter(|text| !text.trim().is_empty())
		{
			parts.push(text.trim().to_owned());
		}

		for child in &self.children {
			if child.props.hidden == Some(true) || child.props.aria().hidden == Some(true) {
				continue;
			}

			let content = child.text_content();

			if !content.is_empty() {
				parts.push(content);
			}
		}

		parts.join(" ")
	}

	/// Replaces the style.
	#[must_use]
	pub fn style(mut self, style: Style) -> Self {
		self.style = style;
		self
	}

	/// Merges into the current style.
	#[must_use]
	pub fn with_style(mut self, edit: impl FnOnce(&mut Style)) -> Self {
		edit(&mut self.style);
		self
	}

	/// Replaces the props.
	#[must_use]
	pub fn props(mut self, props: NativeProps) -> Self {
		self.props = props;
		self
	}

	/// Edits the props in place.
	#[must_use]
	pub fn with_props(mut self, edit: impl FnOnce(&mut NativeProps)) -> Self {
		edit(&mut self.props);
		self
	}

	/// Makes a [`Pressable`](crate::components::Pressable) a link to `href`.
	///
	/// The web renders it as an `<a href>`, so the browser can open it in a
	/// new tab and crawlers can follow it, while a plain click still runs the
	/// `on_press` handler instead of leaving the page. Native platforms keep
	/// it a pressable and ignore the target.
	///
	/// Accepts relative URLs, fragments, and HTTP(S), mailto, tel, or sms URLs.
	/// Empty targets, whitespace, controls, backslashes, and other schemes
	/// are ignored: the web renders an ordinary button with its press handler.
	#[must_use]
	pub fn href(mut self, href: impl Into<String>) -> Self {
		self.props
			.extra
			.insert(NativeProps::HREF.to_owned(), PropValue::Str(href.into()));
		self
	}

	/// Sets the accessible name (`aria-label`): what a screen reader announces
	/// first, overriding visible text. Doubles as the image description on
	/// the web.
	#[must_use]
	pub fn aria_label(mut self, label: impl Into<String>) -> Self {
		self.props.accessibility_label = Some(label.into());
		self
	}

	/// Sets the accessible name; the same as [`Self::aria_label`].
	#[must_use]
	pub fn accessibility_label(self, label: impl Into<String>) -> Self {
		self.aria_label(label)
	}

	/// Sets a stable identifier, unique within the tree, that ARIA
	/// relationships reference and the web renders as the DOM `id`.
	#[must_use]
	pub fn id(mut self, id: impl Into<String>) -> Self {
		self.props.id = Some(id.into());
		self
	}

	/// Sets the ARIA role, overriding the widget kind's implicit role.
	///
	/// On the web, structural roles become semantic elements: a heading with
	/// a level renders as `<h1>` to `<h6>`, a list as `<ul>`, and landmarks as
	/// their HTML elements.
	#[must_use]
	pub fn role(mut self, role: Role) -> Self {
		self.props.aria_mut().role = Some(role);
		self
	}

	/// Names this element after other elements (`aria-labelledby`), by id.
	/// References win over `aria_label`.
	#[must_use]
	pub fn aria_labelled_by<I>(mut self, ids: I) -> Self
	where
		I: IntoIterator,
		I::Item: Into<String>,
	{
		self.props.aria_mut().labelled_by = ids.into_iter().map(Into::into).collect();
		self
	}

	/// Sets a description beyond the name (`aria-description`).
	#[must_use]
	pub fn aria_description(mut self, description: impl Into<String>) -> Self {
		self.props.aria_mut().description = Some(description.into());
		self
	}

	/// Describes this element with other elements (`aria-describedby`), by
	/// id; the usual home of help text and error messages.
	#[must_use]
	pub fn aria_described_by<I>(mut self, ids: I) -> Self
	where
		I: IntoIterator,
		I::Item: Into<String>,
	{
		self.props.aria_mut().described_by = ids.into_iter().map(Into::into).collect();
		self
	}

	/// Sets a human-readable role label (`aria-roledescription`).
	#[must_use]
	pub fn aria_role_description(mut self, description: impl Into<String>) -> Self {
		self.props.aria_mut().role_description = Some(description.into());
		self
	}

	/// Sets the heading, list, or tree depth (`aria-level`), starting at one.
	#[must_use]
	pub fn aria_level(mut self, level: u32) -> Self {
		self.props.aria_mut().level = Some(level);
		self
	}

	/// Sets the one-based position in a set (`aria-posinset`).
	#[must_use]
	pub fn aria_pos_in_set(mut self, position: u32) -> Self {
		self.props.aria_mut().pos_in_set = Some(position);
		self
	}

	/// Sets the number of items in the set (`aria-setsize`).
	#[must_use]
	pub fn aria_set_size(mut self, size: u32) -> Self {
		self.props.aria_mut().set_size = Some(size);
		self
	}

	/// Sets the widget's axis (`aria-orientation`).
	#[must_use]
	pub fn aria_orientation(mut self, orientation: Orientation) -> Self {
		self.props.aria_mut().orientation = Some(orientation);
		self
	}

	/// Hides the element and its descendants from assistive technology while
	/// keeping them visible (`aria-hidden`), for decorative content.
	#[must_use]
	pub fn aria_hidden(mut self, hidden: bool) -> Self {
		self.props.aria_mut().hidden = Some(hidden);
		self
	}

	/// Marks the element as being modified (`aria-busy`).
	#[must_use]
	pub fn aria_busy(mut self, busy: bool) -> Self {
		self.props.aria_mut().busy = Some(busy);
		self
	}

	/// Sets the check state (`aria-checked`) of a custom checkbox, option,
	/// menu item, or switch; `true`, `false`, or [`Tristate::Mixed`].
	#[must_use]
	pub fn aria_checked(mut self, checked: impl Into<Tristate>) -> Self {
		self.props.aria_mut().checked = Some(checked.into());
		self
	}

	/// Sets the pressed state (`aria-pressed`) of a toggle button.
	#[must_use]
	pub fn aria_pressed(mut self, pressed: impl Into<Tristate>) -> Self {
		self.props.aria_mut().pressed = Some(pressed.into());
		self
	}

	/// Sets whether the element is selected (`aria-selected`).
	#[must_use]
	pub fn aria_selected(mut self, selected: bool) -> Self {
		self.props.aria_mut().selected = Some(selected);
		self
	}

	/// Sets whether a control's popup or section is open (`aria-expanded`).
	#[must_use]
	pub fn aria_expanded(mut self, expanded: bool) -> Self {
		self.props.aria_mut().expanded = Some(expanded);
		self
	}

	/// Marks what the element currently represents (`aria-current`).
	#[must_use]
	pub fn aria_current(mut self, current: Current) -> Self {
		self.props.aria_mut().current = Some(current);
		self
	}

	/// Sets whether a dialog blocks the rest of the page (`aria-modal`).
	#[must_use]
	pub fn aria_modal(mut self, modal: bool) -> Self {
		self.props.aria_mut().modal = Some(modal);
		self
	}

	/// Sets whether a field must be filled in (`aria-required`).
	#[must_use]
	pub fn aria_required(mut self, required: bool) -> Self {
		self.props.aria_mut().required = Some(required);
		self
	}

	/// Sets why a field's value is invalid (`aria-invalid`); `true`, `false`,
	/// or a specific [`Invalid`] kind.
	#[must_use]
	pub fn aria_invalid(mut self, invalid: impl Into<Invalid>) -> Self {
		self.props.aria_mut().invalid = Some(invalid.into());
		self
	}

	/// Sets whether a field cannot be edited (`aria-readonly`).
	#[must_use]
	pub fn aria_read_only(mut self, read_only: bool) -> Self {
		self.props.aria_mut().read_only = Some(read_only);
		self
	}

	/// Sets whether a text field accepts multiple lines (`aria-multiline`).
	#[must_use]
	pub fn aria_multiline(mut self, multiline: bool) -> Self {
		self.props.aria_mut().multiline = Some(multiline);
		self
	}

	/// Sets what a control opens (`aria-haspopup`); `true`, `false`, or a
	/// specific [`HasPopup`] kind.
	#[must_use]
	pub fn aria_has_popup(mut self, popup: impl Into<HasPopup>) -> Self {
		self.props.aria_mut().has_popup = Some(popup.into());
		self
	}

	/// Sets a custom range widget's current value (`aria-valuenow`).
	#[must_use]
	pub fn aria_value_now(mut self, value: f64) -> Self {
		self.props.aria_mut().value_now = Some(value);
		self
	}

	/// Sets a custom range widget's minimum (`aria-valuemin`).
	#[must_use]
	pub fn aria_value_min(mut self, value: f64) -> Self {
		self.props.aria_mut().value_min = Some(value);
		self
	}

	/// Sets a custom range widget's maximum (`aria-valuemax`).
	#[must_use]
	pub fn aria_value_max(mut self, value: f64) -> Self {
		self.props.aria_mut().value_max = Some(value);
		self
	}

	/// Sets a human-readable range value (`aria-valuetext`), such as
	/// "50 percent" for a slider at half.
	#[must_use]
	pub fn aria_value_text(mut self, text: impl Into<String>) -> Self {
		self.props.aria_mut().value_text = Some(text.into());
		self
	}

	/// Makes the element a live region (`aria-live`) whose text changes are
	/// announced.
	#[must_use]
	pub fn aria_live(mut self, live: Live) -> Self {
		self.props.aria_mut().live = Some(live);
		self
	}

	/// Sets whether a live region announces as a whole (`aria-atomic`).
	#[must_use]
	pub fn aria_atomic(mut self, atomic: bool) -> Self {
		self.props.aria_mut().atomic = Some(atomic);
		self
	}

	/// Links this element to the elements it controls (`aria-controls`), by
	/// id.
	#[must_use]
	pub fn aria_controls<I>(mut self, ids: I) -> Self
	where
		I: IntoIterator,
		I::Item: Into<String>,
	{
		self.props.aria_mut().controls = ids.into_iter().map(Into::into).collect();
		self
	}

	/// Sets the position in sequential keyboard focus (`tabindex`); zero
	/// joins the natural order and a negative value leaves it.
	#[must_use]
	pub fn tab_index(mut self, index: i32) -> Self {
		self.props.aria_mut().tab_index = Some(index);
		self
	}

	/// Sets any other `aria-*` attribute, emitted verbatim on the web and
	/// ignored by native backends. Attributes with typed builders are
	/// rejected by [`Self::validate`].
	#[must_use]
	pub fn aria_attr(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
		self.props
			.aria_mut()
			.extra
			.insert(name.into(), value.into());
		self
	}

	/// Edits every accessibility property in place.
	#[must_use]
	pub fn with_aria(mut self, edit: impl FnOnce(&mut Aria)) -> Self {
		edit(self.props.aria_mut());
		self
	}

	/// Sets whether a control accepts interaction.
	#[must_use]
	pub fn enabled(mut self, enabled: bool) -> Self {
		self.props.enabled = Some(enabled);
		self
	}

	/// Sets a text input's controlled value.
	#[must_use]
	pub fn value(mut self, value: impl Into<String>) -> Self {
		self.props.value = Some(value.into());
		self
	}

	/// Replaces a `Text`'s content with styled runs that flow and wrap as one
	/// paragraph, such as syntax-highlighted code or a bold word in a
	/// sentence.
	///
	/// The joined runs become the text too, so layout, accessibility, and
	/// backends that draw one style per label see the same characters.
	#[must_use]
	pub fn spans<I>(mut self, spans: I) -> Self
	where
		I: IntoIterator,
		I::Item: Into<crate::TextSpan>,
	{
		let spans: Vec<crate::TextSpan> = spans.into_iter().map(Into::into).collect();
		self.props.text = Some(spans.iter().map(|span| span.text.as_str()).collect());
		self.props.spans = Some(spans);
		self
	}

	/// Asks for keyboard focus while `focused` is true: after each commit,
	/// the platform moves focus to this widget if it is not already there,
	/// the way a controlled value overrides what the user typed. Keep it true
	/// on one widget at a time, such as a chat prompt, and move it to a
	/// dialog's search field while the dialog is open.
	#[must_use]
	pub fn focused(mut self, focused: bool) -> Self {
		self.props.focused = Some(focused);
		self
	}

	/// Sets the hint shown when a text input is empty.
	#[must_use]
	pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
		self.props.placeholder = Some(placeholder.into());
		self
	}

	/// Sets an image's source, understood by the platform backend.
	#[must_use]
	pub fn source(mut self, source: impl Into<String>) -> Self {
		self.props.source = Some(source.into());
		self
	}

	/// Sets whether a switch, checkbox, or radio option is on.
	#[must_use]
	pub fn checked(mut self, checked: bool) -> Self {
		self.props.checked = Some(checked);
		self
	}

	/// Sets the group a radio option belongs to.
	#[must_use]
	pub fn radio_group(mut self, group: RadioGroup) -> Self {
		self.props.radio_group = Some(group);
		self
	}

	/// Sets a slider's value and step grid.
	#[must_use]
	pub fn slider(mut self, state: SliderState) -> Self {
		self.props.slider = Some(state);
		self
	}

	/// Sets a progress indicator's progress.
	#[must_use]
	pub fn progress(mut self, progress: Progress) -> Self {
		self.props.progress = Some(progress);
		self
	}

	/// Sets whether an activity indicator is animating.
	#[must_use]
	pub fn running(mut self, running: bool) -> Self {
		self.props.running = Some(running);
		self
	}

	/// Appends a path to a `Vector`'s drawing, painted over earlier paths.
	#[must_use]
	pub fn path(mut self, path: VectorPath) -> Self {
		self.drawing_mut().push(path);
		self
	}

	/// Appends several paths to a `Vector`'s drawing, in painting order.
	#[must_use]
	pub fn paths(mut self, paths: impl IntoIterator<Item = VectorPath>) -> Self {
		let drawing = self.drawing_mut();

		for path in paths {
			drawing.push(path);
		}

		self
	}

	/// Replaces a `Vector`'s coordinate space.
	#[must_use]
	pub fn view_box(mut self, view_box: ViewBox) -> Self {
		self.drawing_mut().set_view_box(view_box);
		self
	}

	/// Sets how a `Vector`'s view box maps onto its frame.
	#[must_use]
	pub fn scaling(mut self, scaling: VectorScaling) -> Self {
		self.drawing_mut().set_scaling(scaling);
		self
	}

	/// Sets where a fitted or filled `Vector` drawing sits in its frame.
	#[must_use]
	pub fn align(mut self, alignment: VectorAlignment) -> Self {
		self.drawing_mut().set_alignment(alignment);
		self
	}

	/// Replaces a `Vector`'s whole drawing, such as one imported from an SVG
	/// document. Sharing one drawing between elements copies a pointer.
	#[must_use]
	pub fn drawing(mut self, drawing: Drawing) -> Self {
		*self.drawing_mut() = drawing;
		self
	}

	/// The drawing of a `Vector` element; any other kind has no drawing to
	/// edit.
	fn drawing_mut(&mut self) -> &mut Drawing {
		assert!(
			self.kind == NativeKind::Vector,
			"paths, view boxes, and scaling apply only to Vector elements, not {}",
			self.kind
		);
		self.props.drawing.get_or_insert_with(Drawing::default)
	}

	/// Sets the background color.
	#[must_use]
	pub fn background(mut self, color: Color) -> Self {
		self.props.background = Some(color);
		self
	}

	/// Sets the foreground (text) color.
	#[must_use]
	pub fn color(mut self, color: Color) -> Self {
		self.props.color = Some(color);
		self
	}

	/// Rounds the corners, clipping the content to the rounded shape.
	#[must_use]
	pub fn corner_radius(mut self, radius: f32) -> Self {
		self.props.corner_radius = Some(radius);
		self
	}

	/// Draws a border stroke of the given width and color.
	///
	/// The stroke is visual only; set the style's `border` as well when it
	/// should take layout space.
	#[must_use]
	pub fn border(mut self, width: f32, color: Color) -> Self {
		self.props.border_width = Some(width);
		self.props.border_color = Some(color);
		self
	}

	/// Sets the opacity of the widget and its content.
	#[must_use]
	pub fn opacity(mut self, opacity: f32) -> Self {
		self.props.opacity = Some(opacity);
		self
	}

	/// Sets the font size and weight.
	#[must_use]
	pub fn font(mut self, size: f32, weight: FontWeight) -> Self {
		self.props.font_size = Some(size);
		self.props.font_weight = Some(weight);
		self
	}

	/// Sets the horizontal text alignment.
	#[must_use]
	pub fn text_align(mut self, align: TextAlign) -> Self {
		self.props.text_align = Some(align);
		self
	}

	/// Appends a child.
	///
	/// A `Text` or `Button` takes plain text children (unstyled `Text`
	/// elements with no children or handlers) as more of its content, so
	/// `Text("Count: ").child(Text("3"))` reads "Count: 3".
	#[must_use]
	pub fn child(mut self, child: Element) -> Self {
		self.push_child(child);
		self
	}

	/// Appends every child, joining plain text into a `Text` or `Button` as
	/// [`Self::child`] does.
	#[must_use]
	pub fn children(mut self, children: impl IntoIterator<Item = Element>) -> Self {
		let children = children.into_iter();
		// Growing the list moves every element already in it, and elements
		// are large; reserving once spares that, above all in the guest's
		// interpreter.
		self.children.reserve(children.size_hint().0);

		for child in children {
			self.push_child(child);
		}
		self
	}

	fn push_child(&mut self, child: Element) {
		if matches!(self.kind, NativeKind::Text | NativeKind::Button) && child.is_plain_text() {
			let text = child.props.text.unwrap_or_default();
			self.props.text.get_or_insert_default().push_str(&text);
		} else {
			self.children.push(child);
		}
	}

	/// Whether this is text with nothing but its content: no style, classes,
	/// props, children, or handlers.
	fn is_plain_text(&self) -> bool {
		self.kind == NativeKind::Text
			&& self.children.is_empty()
			&& self.handlers.is_empty()
			&& self.classes == Tailwind::default()
			&& self.style == Style::default()
			&& NativeProps {
				text: None,
				..self.props.clone()
			} == NativeProps::default()
	}

	/// Registers a handler for one event slot.
	#[must_use]
	pub fn on(
		mut self,
		slot: NativeEventSlot,
		handler: impl Fn(&NativeEvent) -> crate::props::EventOutcome + Send + Sync + 'static,
	) -> Self {
		self.handlers.push(HandlerEntry {
			event: slot,
			handler: Arc::new(handler),
		});
		self
	}

	/// Registers a press handler, the most common case.
	#[must_use]
	pub fn on_press(self, handler: impl Fn() + Send + Sync + 'static) -> Self {
		self.on(NativeEventSlot::Press, move |_| {
			handler();
			crate::props::EventOutcome::Handled
		})
	}

	/// Handles key presses while focus is inside this element, before the
	/// focused widget acts on them. Return [`EventOutcome::Handled`] to keep
	/// the key from the widget: a root view that takes ctrl+p for a command
	/// palette while a text field is focused.
	///
	/// [`EventOutcome::Handled`]: crate::EventOutcome::Handled
	#[must_use]
	pub fn on_key(
		self,
		handler: impl Fn(&crate::KeyPress) -> crate::EventOutcome + Send + Sync + 'static,
	) -> Self {
		self.on(NativeEventSlot::Key, move |event| {
			match event {
				NativeEvent::Key(key) => handler(key),
				_ => crate::EventOutcome::Unhandled,
			}
		})
	}

	/// Handles an editable field's new value.
	#[must_use]
	pub fn on_change(self, handler: impl Fn(&str) + Send + Sync + 'static) -> Self {
		self.on(NativeEventSlot::Change, move |event| {
			if let NativeEvent::Change { value } = event {
				handler(value);
			}

			crate::props::EventOutcome::Handled
		})
	}

	/// Handles a boolean control's proposed checked state.
	#[must_use]
	pub fn on_toggle(self, handler: impl Fn(bool) + Send + Sync + 'static) -> Self {
		self.on(NativeEventSlot::Toggle, move |event| {
			if let NativeEvent::Toggle { checked } = event {
				handler(*checked);
			}
			crate::props::EventOutcome::Handled
		})
	}

	/// Handles a slider's proposed value on its configured step grid.
	#[must_use]
	pub fn on_value_change(self, handler: impl Fn(f64) + Send + Sync + 'static) -> Self {
		self.on(NativeEventSlot::ValueChange, move |event| {
			if let NativeEvent::ValueChange { value } = event {
				handler(*value);
			}
			crate::props::EventOutcome::Handled
		})
	}

	/// The handlers registered on this element.
	pub(crate) fn handlers(&self) -> &[HandlerEntry] {
		&self.handlers
	}
}
