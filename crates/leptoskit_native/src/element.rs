//! The declarative element tree apps build each frame.

use std::sync::Arc;

use leptoskit_yoga::Style;

use crate::props::Color;
use crate::props::EventHandler;
use crate::props::FontWeight;
use crate::props::NativeEvent;
use crate::props::NativeKind;
use crate::props::NativeProps;
use crate::props::TextAlign;
use crate::tailwind::ClassError;
use crate::tailwind::StyleContext;
use crate::tailwind::Tailwind;

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

impl Element {
	/// Checks structural control contracts before any backend mutations occur.
	///
	/// Radio names are scoped to this complete tree. At most one option may be
	/// checked in each group; the application owns that selection.
	pub fn validate(&self) -> Result<(), crate::controls::ControlTreeError> {
		use crate::controls::ControlTreeError;

		let mut pending = vec![(self, None)];
		let mut selected_groups = std::collections::BTreeSet::new();

		while let Some((element, button_ancestor)) = pending.pop() {
			if let Some(container) = button_ancestor {
				if matches!(
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
							|| matches!(element.kind, NativeKind::Custom(_))))
				{
					return Err(ControlTreeError::InteractiveDescendant {
						container,
						descendant: element.kind,
					});
				}
			}

			if element.kind == NativeKind::Pressable
				&& element
					.props
					.accessibility_label
					.as_deref()
					.is_none_or(|label| label.trim().is_empty())
			{
				return Err(ControlTreeError::MissingPressableLabel);
			}

			if matches!(
				element.kind,
				NativeKind::Image
					| NativeKind::TextInput
					| NativeKind::PasswordInput
					| NativeKind::Switch
					| NativeKind::Checkbox
					| NativeKind::Radio
					| NativeKind::Slider
					| NativeKind::LinearProgressIndicator
					| NativeKind::ActivityIndicator
			) && !element.children.is_empty()
			{
				return Err(ControlTreeError::Children { kind: element.kind });
			}

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
	/// viewport changes do not require rebuilding the screen.
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
		let combined = format!("{} {}", self.classes.classes(), classes.as_ref());
		self.classes = Tailwind::parse(&combined)?;
		Ok(self)
	}

	/// The parsed utility program shared by native and web rendering.
	pub fn classes(&self) -> &Tailwind {
		&self.classes
	}

	/// Resolves this tree for an environment, including inherited text styles.
	///
	/// The source tree remains unchanged. The returned snapshot retains no
	/// classes: it is ready for backend reconciliation or serialization.
	#[must_use]
	pub fn resolved(&self, context: &StyleContext) -> Self {
		self.resolve_inherited(context, &NativeProps::default())
	}

	fn resolve_inherited(&self, context: &StyleContext, parent: &NativeProps) -> Self {
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
		element.props.text_align = element.props.text_align.or(parent.text_align);

		if parent.enabled == Some(false) {
			element.props.enabled = Some(false);
		}

		self.classes
			.apply(&mut element.style, &mut element.props, context);

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
		element.children = self
			.children
			.iter()
			.map(|child| child.resolve_inherited(context, &element.props))
			.collect();
		element
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

	/// Sets a screen-reader label, or an image description on the web.
	#[must_use]
	pub fn accessibility_label(mut self, label: impl Into<String>) -> Self {
		self.props.accessibility_label = Some(label.into());
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

	/// Sets the hint shown when a text input is empty.
	#[must_use]
	pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
		self.props.placeholder = Some(placeholder.into());
		self
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
	#[must_use]
	pub fn child(mut self, child: Element) -> Self {
		self.children.push(child);
		self
	}

	/// Appends every child.
	#[must_use]
	pub fn children(mut self, children: impl IntoIterator<Item = Element>) -> Self {
		self.children.extend(children);
		self
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
