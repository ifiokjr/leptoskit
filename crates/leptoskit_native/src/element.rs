//! The declarative element tree apps build each frame.

use std::sync::Arc;

use leptoskit_yoga::Style;

use crate::props::EventHandler;
use crate::props::NativeEvent;
use crate::props::NativeKind;
use crate::props::NativeProps;

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
	/// Creates an element of the given kind with default style and props.
	pub fn new(kind: NativeKind) -> Self {
		Self {
			kind,
			style: Style::default(),
			props: NativeProps::default(),
			children: Vec::new(),
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

	/// A `TextInput` element.
	pub fn text_input() -> Self {
		Self::new(NativeKind::TextInput)
	}

	/// A `Button` element with the given label.
	pub fn button(label: impl Into<String>) -> Self {
		let mut element = Self::new(NativeKind::Button);
		element.props.text = Some(label.into());
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

	/// The handlers registered on this element.
	pub(crate) fn handlers(&self) -> &[HandlerEntry] {
		&self.handlers
	}
}
