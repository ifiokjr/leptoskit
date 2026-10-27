//! The AppKit implementation of the native backend contract.

use std::rc::Rc;
use std::sync::Arc;

use leptoskit_native::Mutation;
use leptoskit_native::MutationList;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::Progress;
use leptoskit_native::SharedTextMeasurer;
use leptoskit_native::TextAlign;
use leptoskit_native::TextStyle;
use leptoskit_yoga::NodeId;
use objc2::AnyThread;
use objc2::MainThreadMarker;
use objc2::MainThreadOnly;
use objc2::Message;
use objc2::rc::Retained;
use objc2::rc::Weak;
use objc2::rc::autoreleasepool;
use objc2::sel;
use objc2_app_kit::NSAppearance;
use objc2_app_kit::NSAppearanceCustomization;
use objc2_app_kit::NSAppearanceNameAqua;
use objc2_app_kit::NSAppearanceNameDarkAqua;
use objc2_app_kit::NSBezelStyle;
use objc2_app_kit::NSButton;
use objc2_app_kit::NSButtonType;
use objc2_app_kit::NSColor;
use objc2_app_kit::NSFocusRingType;
use objc2_app_kit::NSImage;
use objc2_app_kit::NSImageScaling;
use objc2_app_kit::NSImageView;
use objc2_app_kit::NSLineBreakMode;
use objc2_app_kit::NSProgressIndicator;
use objc2_app_kit::NSProgressIndicatorStyle;
use objc2_app_kit::NSScrollView;
use objc2_app_kit::NSSecureTextField;
use objc2_app_kit::NSSlider;
use objc2_app_kit::NSSwitch;
use objc2_app_kit::NSTextField;
use objc2_app_kit::NSTextFieldBezelStyle;
use objc2_app_kit::NSView;
use objc2_app_kit::NSViewBoundsDidChangeNotification;
use objc2_app_kit::NSWindowOrderingMode;
use objc2_app_kit::NSWritingDirection;
use objc2_foundation::NSNotificationCenter;
use objc2_foundation::NSPoint;
use objc2_foundation::NSRect;
use objc2_foundation::NSSize;
use objc2_foundation::NSString;
use objc2_quartz_core::kCACornerCurveCircular;
use objc2_quartz_core::kCACornerCurveContinuous;

use crate::measure::AppKitTextMeasurer;
use crate::pressable::content_button;
use crate::text::attributed_string;
use crate::text::ns_alignment;
use crate::text::ns_color;
use crate::text::system_font;
use crate::text::text_color;
use crate::views::ActionTarget;
use crate::views::ControlledSlider;
pub use crate::views::EventListener;
use crate::views::EventSink;
use crate::views::EventSuppression;
use crate::views::FlippedView;
use crate::views::PressButton;
use crate::views::RadioButton;
use crate::views::RadioMember;

/// A [`NativeBackend`] that renders into real AppKit views.
///
/// Every node becomes one `NSView` (see the [crate docs](crate) for the
/// mapping). The backend owns the views; [`view`](Self::view) hands out the
/// one for a node, typically the root, to put in a window or snapshot.
///
/// # Events
///
/// Controls report user interaction through one listener, installed with
/// [`set_event_listener`](Self::set_event_listener):
///
/// - a `Button` click delivers [`NativeEvent::Press`];
/// - a `TextInput` delivers [`NativeEvent::Change`] on every edit,
///   [`NativeEvent::Submit`] on return, and [`NativeEvent::Focus`] /
///   [`NativeEvent::Blur`] as editing begins and ends.
/// - `Switch`, `Checkbox`, and `Radio` deliver [`NativeEvent::Toggle`];
/// - `Slider` delivers [`NativeEvent::ValueChange`] on its shared step grid;
/// - `ScrollView` delivers [`NativeEvent::Scroll`] with the content offset
///   whenever its content moves;
/// - `PasswordInput` uses the same editing events as `TextInput`.
///
/// The listener receives the node id the surface committed, so the usual
/// listener forwards to
/// [`NativeSurface::dispatch_event`](leptoskit_native::NativeSurface::dispatch_event)
/// and re-renders. It runs on the main thread from inside the AppKit event
/// loop, never during [`apply`](NativeBackend::apply), so it may freely
/// borrow the surface that owns this backend. Events that arrive while no
/// listener is installed are dropped.
///
/// The backend is `!Send`: AppKit views live on the main thread, which the
/// [`MainThreadMarker`] passed to [`new`](Self::new) proves.
pub struct AppKitBackend {
	mtm: MainThreadMarker,
	/// Widgets indexed by node index; deleted nodes leave `None`.
	widgets: Vec<Option<Widget>>,
	/// Scroll views, whose document size follows their children.
	scroll_views: Vec<NodeId>,
	sink: Rc<EventSink>,
	measurer: SharedTextMeasurer,
	applying: bool,
	appearance: Option<Retained<NSAppearance>>,
}

impl std::fmt::Debug for AppKitBackend {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("AppKitBackend")
			.field("widgets", &self.widgets.iter().flatten().count())
			.finish_non_exhaustive()
	}
}

/// One node's views.
struct Widget {
	id: NodeId,
	kind: NativeKind,
	/// The view positioned inside the parent.
	view: Retained<NSView>,
	/// The kind-specific control behind `view`.
	control: Control,
	/// The children, in order.
	children: Vec<NodeId>,
	/// The live-region text announced for this widget, if any, so changes
	/// post exactly one announcement.
	announced: Option<String>,
}

impl Widget {
	/// The view children are added to.
	fn content(&self) -> &NSView {
		match &self.control {
			Control::Scroll { document, .. } => document,
			_ => &self.view,
		}
	}
}

/// The concrete AppKit control of a widget.
enum Control {
	/// A plain flipped container.
	Container,
	/// A text label.
	Label(Retained<NSTextField>),
	/// A button and the target its clicks go to. Controls hold their target
	/// weakly, so the widget keeps it alive.
	Button {
		button: Retained<NSButton>,
		target: Retained<ActionTarget>,
	},
	/// Native button tracking over a flipped decorative content hierarchy.
	Pressable {
		button: Retained<NSButton>,
		target: Retained<ActionTarget>,
	},
	/// A checkbox or radio button retains its controlled action target.
	Toggle {
		button: Retained<NSButton>,
		target: Retained<ActionTarget>,
		radio: bool,
	},
	/// An on/off switch, using the native accessibility and keyboard behavior.
	Switch {
		switch: Retained<NSSwitch>,
		target: Retained<ActionTarget>,
	},
	/// A continuous slider whose proposals use the shared range and step grid.
	Slider {
		slider: Retained<NSSlider>,
		target: Retained<ActionTarget>,
	},
	/// A native loading animation or determinate progress bar.
	Progress {
		indicator: Retained<NSProgressIndicator>,
		requested: bool,
		animating: bool,
	},
	/// An editable text field, with the target of its action and delegate.
	Input {
		field: Retained<NSTextField>,
		target: Retained<ActionTarget>,
	},
	/// An image view and the source it last loaded.
	Image {
		image_view: Retained<NSImageView>,
		source: Option<String>,
	},
	/// A scroll view, its flipped document view, and the target that
	/// observes its clip view's scrolling.
	Scroll {
		scroll: Retained<NSScrollView>,
		document: Retained<FlippedView>,
		target: Retained<ActionTarget>,
	},
}

impl AppKitBackend {
	/// A backend with no views, measuring text with
	/// [`AppKitTextMeasurer`].
	pub fn new(mtm: MainThreadMarker) -> Self {
		Self {
			mtm,
			widgets: Vec::new(),
			scroll_views: Vec::new(),
			sink: Rc::new(EventSink::default()),
			measurer: Arc::new(AppKitTextMeasurer::new()),
			applying: false,
			appearance: None,
		}
	}

	/// The main-thread marker the backend was created with.
	pub fn main_thread_marker(&self) -> MainThreadMarker {
		self.mtm
	}

	/// Installs the listener controls deliver their events to, replacing any
	/// previous one. See [the type docs](Self#events).
	pub fn set_event_listener(&self, listener: impl Fn(NodeId, &NativeEvent) + 'static) {
		self.sink.set_listener(Some(Rc::new(listener)));
	}

	/// Removes the event listener; later events are dropped.
	pub fn clear_event_listener(&self) {
		self.sink.set_listener(None);
	}

	/// Holds back widget events while the backend itself drives AppKit, so
	/// the notifications AppKit posts synchronously never re-enter a host
	/// that is still inside its call.
	pub(crate) fn suppress_events(&self) -> EventSuppression {
		self.sink.suppress()
	}

	/// The view of a live node: the one positioned inside its parent.
	pub fn view(&self, id: NodeId) -> Option<&NSView> {
		self.widget(id).map(|widget| &*widget.view)
	}

	/// Whether the backend has started a live indicator's native animation.
	///
	/// Hidden ancestors, a stopped activity indicator, determinate progress,
	/// and deletion all stop the animation. Other node kinds return `None`.
	pub fn is_animating(&self, id: NodeId) -> Option<bool> {
		match &self.widget(id)?.control {
			Control::Progress { animating, .. } => Some(*animating),
			_ => None,
		}
	}

	/// Sends `NativeEvent::Press` from a `Button` node as if it were clicked,
	/// through the same action a real click triggers. Returns `false` when
	/// the node is not a live button.
	pub fn perform_press(&self, id: NodeId) -> bool {
		match self.widget(id).map(|widget| &widget.control) {
			Some(Control::Button { button, .. } | Control::Pressable { button, .. }) => {
				// SAFETY: `performClick:` accepts any sender, including none.
				unsafe { button.performClick(None) };
				true
			}
			_ => false,
		}
	}

	fn widget(&self, id: NodeId) -> Option<&Widget> {
		self.widgets.get(id.index()).and_then(Option::as_ref)
	}

	/// The widget a mutation names.
	///
	/// The renderer only names nodes it created and has not deleted, so a
	/// miss is a renderer bug and panics rather than corrupting the tree.
	fn expect_widget(&self, id: NodeId) -> &Widget {
		self.widget(id).unwrap_or_else(|| {
			panic!("mutation names {id:?}, which the AppKit backend does not hold")
		})
	}

	fn expect_widget_mut(&mut self, id: NodeId) -> &mut Widget {
		self.widgets
			.get_mut(id.index())
			.and_then(Option::as_mut)
			.unwrap_or_else(|| {
				panic!("mutation names {id:?}, which the AppKit backend does not hold")
			})
	}

	fn build(&self, id: NodeId, kind: NativeKind) -> Widget {
		let mtm = self.mtm;
		let target = || ActionTarget::new(mtm, id, Rc::clone(&self.sink));
		let (view, control): (Retained<NSView>, Control) = match kind {
			NativeKind::View | NativeKind::Custom(_) => {
				(
					Retained::into_super(FlippedView::new(mtm)),
					Control::Container,
				)
			}
			NativeKind::Text | NativeKind::TextRun => build_label(mtm),
			NativeKind::Button | NativeKind::Pressable => build_button(mtm, kind, target()),
			NativeKind::Switch => build_switch(mtm, target()),
			NativeKind::Checkbox | NativeKind::Radio => build_toggle(mtm, kind, target()),
			NativeKind::Slider => build_slider(mtm, target()),
			NativeKind::LinearProgressIndicator | NativeKind::ActivityIndicator => {
				build_progress(mtm, kind)
			}
			NativeKind::TextInput | NativeKind::PasswordInput => build_input(mtm, kind, target()),
			NativeKind::Image => build_image(mtm),
			NativeKind::ScrollView => build_scroll(mtm, target()),
		};
		view.setAppearance(self.appearance.as_deref());
		Widget {
			id,
			kind,
			view,
			control,
			children: Vec::new(),
			announced: None,
		}
	}

	/// Sizes each scroll view's document to the union of its children, so
	/// the whole content scrolls.
	fn refresh_scroll_documents(&self) {
		for &id in &self.scroll_views {
			let Some(widget) = self.widget(id) else {
				continue;
			};
			let Control::Scroll {
				scroll, document, ..
			} = &widget.control
			else {
				continue;
			};
			let visible = scroll.contentSize();
			let mut extent = NSSize::new(visible.width, visible.height);
			for child in &document.subviews() {
				let frame = child.frame();
				extent.width = extent.width.max(frame.origin.x + frame.size.width);
				extent.height = extent.height.max(frame.origin.y + frame.size.height);
			}
			document.setFrame(NSRect::new(NSPoint::ZERO, extent));
		}
	}

	/// Builds radio navigation order from the committed view tree, across
	/// wrappers.
	fn refresh_radios(&self) {
		let mut children = std::collections::HashSet::new();

		for widget in self.widgets.iter().flatten() {
			children.extend(widget.children.iter().copied());
		}

		let mut stack: Vec<_> = self
			.widgets
			.iter()
			.flatten()
			.filter_map(|widget| (!children.contains(&widget.id)).then_some(widget.id))
			.collect();
		stack.reverse();
		let mut radios = Vec::new();

		while let Some(id) = stack.pop() {
			let widget = self.expect_widget(id);

			if let Control::Toggle {
				button,
				target,
				radio: true,
			} = &widget.control
			{
				radios.push(RadioMember {
					button: Weak::new(button),
					target: Weak::new(target),
				});
			}

			stack.extend(widget.children.iter().rev().copied());
		}

		self.sink.set_radios(radios);
	}

	fn refresh_animations(&mut self) {
		for widget in self.widgets.iter_mut().flatten() {
			if let Control::Progress {
				indicator,
				requested,
				animating,
			} = &mut widget.control
			{
				let next = *requested && !indicator.isHiddenOrHasHiddenAncestor();

				if next != *animating {
					// SAFETY: these standard actions accept an optional sender,
					// and the backend owns the indicator
					// throughout its animation.
					unsafe {
						if next {
							indicator.startAnimation(None);
						} else {
							indicator.stopAnimation(None);
						}
					}
					*animating = next;
				}
			}
		}
	}

	fn refresh_controls_if_unbatched(&mut self) {
		if !self.applying {
			self.refresh_radios();
			self.refresh_animations();
		}
	}
}

/// Frames snap to this many divisions per point: half points, the device
/// pixels of a Retina display.
const PIXELS_PER_POINT: f64 = 2.0;

/// Snaps a layout frame to the pixel grid.
///
/// Layout produces fractional frames (a third of a row, say). Snapping both
/// edges to the grid keeps the gaps between siblings even, and keeps AppKit
/// from snapping on its own, which can shave a fraction off a label and wrap
/// its last word. A view whose size was measured from its text is never
/// narrower or shorter than measured, so its text always fits.
fn pixel_aligned(x: f32, y: f32, width: f32, height: f32, measured: bool) -> NSRect {
	let snap = |value: f64| (value * PIXELS_PER_POINT).round() / PIXELS_PER_POINT;
	let snap_up = |value: f64| (value * PIXELS_PER_POINT).ceil() / PIXELS_PER_POINT;
	let (x, y, width, height) = (
		f64::from(x),
		f64::from(y),
		f64::from(width),
		f64::from(height),
	);
	let left = snap(x);
	let top = snap(y);
	let mut snapped_width = snap(x + width) - left;
	let mut snapped_height = snap(y + height) - top;
	if measured {
		snapped_width = snapped_width.max(snap_up(width));
		snapped_height = snapped_height.max(snap_up(height));
	}
	NSRect::new(
		NSPoint::new(left, top),
		NSSize::new(snapped_width, snapped_height),
	)
}

/// A non-editable, wrapping label for `Text` and `TextRun`.
fn build_label(mtm: MainThreadMarker) -> (Retained<NSView>, Control) {
	let label = NSTextField::wrappingLabelWithString(&NSString::new(), mtm);
	label.setEditable(false);
	label.setSelectable(false);
	label.setBezeled(false);
	label.setBordered(false);
	label.setDrawsBackground(false);
	label.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
	label.setUsesSingleLineMode(false);
	label.setMaximumNumberOfLines(0);
	label.setWantsLayer(true);
	(label_view(&label), Control::Label(label))
}

/// A push button for `Button`, or a content-hosting button for `Pressable`,
/// whose clicks go to `target`.
fn build_button(
	mtm: MainThreadMarker,
	kind: NativeKind,
	target: Retained<ActionTarget>,
) -> (Retained<NSView>, Control) {
	let button = if kind == NativeKind::Pressable {
		content_button(mtm, &target)
	} else {
		Retained::into_super(PressButton::new(mtm, &target))
	};
	button.setButtonType(NSButtonType::MomentaryPushIn);
	button.setBezelStyle(NSBezelStyle::Push);
	// SAFETY: `press:` is implemented by `ActionTarget` and takes
	// the sender, as an action must; the widget keeps the target
	// alive for as long as the button can send to it.
	unsafe {
		button.setTarget(Some(&target));
		button.setAction(Some(sel!(press:)));
	}
	button.setWantsLayer(true);
	let view = Retained::into_super(Retained::into_super(button.retain()));
	let control = if kind == NativeKind::Pressable {
		Control::Pressable { button, target }
	} else {
		Control::Button { button, target }
	};
	(view, control)
}

/// An on/off switch whose toggles go to `target`.
fn build_switch(
	mtm: MainThreadMarker,
	target: Retained<ActionTarget>,
) -> (Retained<NSView>, Control) {
	let switch = NSSwitch::initWithFrame(NSSwitch::alloc(mtm), NSRect::ZERO);
	// SAFETY: the retained target implements toggle: with an action
	// signature.
	unsafe {
		switch.setTarget(Some(&target));
		switch.setAction(Some(sel!(toggle:)));
	}
	switch.setWantsLayer(true);
	let view = Retained::into_super(Retained::into_super(switch.retain()));
	(view, Control::Switch { switch, target })
}

/// A checkbox, or a radio button for `Radio`, whose toggles go to `target`.
fn build_toggle(
	mtm: MainThreadMarker,
	kind: NativeKind,
	target: Retained<ActionTarget>,
) -> (Retained<NSView>, Control) {
	let radio = kind == NativeKind::Radio;
	let button = if radio {
		Retained::into_super(RadioButton::new(mtm, &target))
	} else {
		NSButton::initWithFrame(NSButton::alloc(mtm), NSRect::ZERO)
	};
	button.setButtonType(if radio {
		NSButtonType::Radio
	} else {
		NSButtonType::Switch
	});
	button.setAllowsMixedState(false);
	// SAFETY: the retained target implements toggle: with an action
	// signature.
	unsafe {
		button.setTarget(Some(&target));
		button.setAction(Some(sel!(toggle:)));
	}
	button.setWantsLayer(true);
	let view = Retained::into_super(Retained::into_super(button.retain()));
	(
		view,
		Control::Toggle {
			button,
			target,
			radio,
		},
	)
}

/// A continuous horizontal slider whose proposals go to `target`.
fn build_slider(
	mtm: MainThreadMarker,
	target: Retained<ActionTarget>,
) -> (Retained<NSView>, Control) {
	let slider = Retained::into_super(ControlledSlider::new(mtm, &target));
	slider.setContinuous(true);
	slider.setVertical(false);
	// SAFETY: the retained target implements slide: with an action
	// signature.
	unsafe {
		slider.setTarget(Some(&target));
		slider.setAction(Some(sel!(slide:)));
	}
	slider.setWantsLayer(true);
	let view = Retained::into_super(Retained::into_super(slider.retain()));
	(view, Control::Slider { slider, target })
}

/// A spinner for `ActivityIndicator`, or a bar over `0..=1` for
/// `LinearProgressIndicator`, not yet animating.
fn build_progress(mtm: MainThreadMarker, kind: NativeKind) -> (Retained<NSView>, Control) {
	let indicator =
		NSProgressIndicator::initWithFrame(NSProgressIndicator::alloc(mtm), NSRect::ZERO);
	indicator.setMinValue(0.0);
	indicator.setMaxValue(1.0);
	indicator.setStyle(if kind == NativeKind::ActivityIndicator {
		NSProgressIndicatorStyle::Spinning
	} else {
		NSProgressIndicatorStyle::Bar
	});
	indicator.setDisplayedWhenStopped(kind != NativeKind::ActivityIndicator);
	indicator.setWantsLayer(true);
	let view = Retained::into_super(indicator.retain());
	(
		view,
		Control::Progress {
			indicator,
			requested: false,
			animating: false,
		},
	)
}

/// A single-line text field, secure for `PasswordInput`, whose edits and
/// submissions go to `target`.
fn build_input(
	mtm: MainThreadMarker,
	kind: NativeKind,
	target: Retained<ActionTarget>,
) -> (Retained<NSView>, Control) {
	let field = if kind == NativeKind::PasswordInput {
		Retained::into_super(NSSecureTextField::initWithFrame(
			NSSecureTextField::alloc(mtm),
			NSRect::ZERO,
		))
	} else {
		NSTextField::textFieldWithString(&NSString::new(), mtm)
	};
	field.setBezelStyle(NSTextFieldBezelStyle::RoundedBezel);
	field.setUsesSingleLineMode(true);
	field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
	// SAFETY: `submit:` is implemented by `ActionTarget` and takes
	// the sender, and `ActionTarget` conforms to
	// `NSTextFieldDelegate`; the widget keeps the target alive for
	// as long as the field can message it.
	unsafe {
		field.setTarget(Some(&target));
		field.setAction(Some(sel!(submit:)));
		field.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*target)));
	}
	field.setWantsLayer(true);
	(label_view(&field), Control::Input { field, target })
}

/// An image view that scales its image to fit, with nothing loaded yet.
fn build_image(mtm: MainThreadMarker) -> (Retained<NSView>, Control) {
	let image_view = NSImageView::initWithFrame(NSImageView::alloc(mtm), NSRect::ZERO);
	image_view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
	image_view.setWantsLayer(true);
	let view = Retained::into_super(Retained::into_super(image_view.retain()));
	(
		view,
		Control::Image {
			image_view,
			source: None,
		},
	)
}

/// A scroll view over a flipped document view, reporting its scrolling to
/// `target`.
fn build_scroll(
	mtm: MainThreadMarker,
	target: Retained<ActionTarget>,
) -> (Retained<NSView>, Control) {
	let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), NSRect::ZERO);
	scroll.setHasVerticalScroller(true);
	scroll.setHasHorizontalScroller(true);
	scroll.setAutohidesScrollers(true);
	scroll.setDrawsBackground(false);
	let document = FlippedView::new(mtm);
	scroll.setDocumentView(Some(&document));
	scroll.setWantsLayer(true);
	observe_scrolling(&scroll, &target);
	let view = Retained::into_super(scroll.retain());
	(
		view,
		Control::Scroll {
			scroll,
			document,
			target,
		},
	)
}

/// A text field as a plain view.
fn label_view(field: &NSTextField) -> Retained<NSView> {
	Retained::into_super(Retained::into_super(field.retain()))
}

/// Applies the visual props every widget shares: background, corners,
/// border, clipping, and opacity on the layer, plus visibility and
/// accessibility.
fn apply_common(view: &NSView, props: &NativeProps, kind: NativeKind) {
	view.setHidden(props.hidden.unwrap_or(false));
	view.setAlphaValue(f64::from(props.opacity.unwrap_or(1.0).clamp(0.0, 1.0)));
	crate::accessibility::apply(view, props, kind);
	let Some(layer) = view.layer() else {
		return;
	};
	let background = props.background.map(|color| ns_color(color).CGColor());
	layer.setBackgroundColor(background.as_deref());
	let radius = f64::from(props.corner_radius.unwrap_or(0.0).max(0.0));
	layer.setCornerRadius(radius);
	sync_corner_curve(view, kind);
	layer.setMasksToBounds(radius > 0.0 || props.clip.unwrap_or(false));
	layer.setBorderWidth(f64::from(props.border_width.unwrap_or(0.0).max(0.0)));
	let border = props.border_color.map(|color| ns_color(color).CGColor());
	layer.setBorderColor(border.as_deref());
}

/// Picks the corner curve for the view's radius at its current size.
///
/// Continuous corners (the smooth "squircle" curve) suit cards and buttons,
/// but a radius of half the side would draw a lumpy circle with them, so a
/// fully rounded view gets circular corners.
fn sync_corner_curve(view: &NSView, kind: NativeKind) {
	let Some(layer) = view.layer() else {
		return;
	};
	let size = view.frame().size;
	let fully_rounded = layer.cornerRadius() >= size.width.min(size.height) / 2.0 - 0.5;
	// SAFETY: both curve names are immutable `NSString` statics exported by
	// QuartzCore.
	let curve = unsafe {
		if fully_rounded || kind == NativeKind::Pressable {
			kCACornerCurveCircular
		} else {
			kCACornerCurveContinuous
		}
	};
	layer.setCornerCurve(curve);
}

/// How strongly a placeholder shows its field's text color.
const PLACEHOLDER_OPACITY: f64 = 0.45;

fn apply_control(control: &mut Control, props: &NativeProps) {
	let enabled = props.enabled.unwrap_or(true);
	match control {
		Control::Container | Control::Scroll { .. } => {}
		Control::Label(label) => {
			let text = props.text.as_deref().unwrap_or_default();
			label.setAttributedStringValue(&attributed_string(
				text,
				props.text_style(),
				&text_color(props.color),
			));
		}
		Control::Switch { switch, target } => {
			target.sync_props(props);
			switch.setState(isize::from(props.checked.unwrap_or(false)));
			switch.setEnabled(enabled);
		}
		Control::Toggle { button, target, .. } => {
			target.sync_props(props);
			button.setState(isize::from(props.checked.unwrap_or(false)));
			button.setTitle(&NSString::from_str(
				props.text.as_deref().unwrap_or_default(),
			));
			button.setFont(Some(&system_font(props.text_style())));
			button.setEnabled(enabled);
		}
		Control::Slider { slider, target } => {
			target.sync_props(props);
			let state = props.slider.unwrap_or_default();
			slider.setMinValue(state.min());
			slider.setMaxValue(state.max());
			slider.setDoubleValue(state.value());
			slider.setAltIncrementValue(state.step().unwrap_or((state.max() - state.min()) / 10.0));
			slider.setEnabled(enabled);
		}
		Control::Progress {
			indicator,
			requested,
			..
		} => {
			let spinning = indicator.style() == NSProgressIndicatorStyle::Spinning;
			let progress = props.progress.unwrap_or_default();
			let indeterminate = spinning || matches!(progress, Progress::Indeterminate);
			indicator.setIndeterminate(indeterminate);

			if let Progress::Determinate(value) = progress {
				indicator.setDoubleValue(value.get());
			}

			*requested = if spinning {
				props.running.unwrap_or(true)
			} else {
				indeterminate
			};
			indicator.setHidden(props.hidden.unwrap_or(false) || (spinning && !*requested));
		}
		Control::Button { button, .. } => {
			let custom = props.is_custom_drawn();
			button.setBordered(!custom);
			let text = props.text.as_deref().unwrap_or_default();
			// Button titles center unless told otherwise.
			let style = TextStyle {
				text_align: props.text_align.unwrap_or(TextAlign::Center),
				..props.text_style()
			};
			button.setAttributedTitle(&attributed_string(text, style, &text_color(props.color)));
			button.setEnabled(enabled);
		}
		Control::Pressable { button, .. } => {
			button.setBordered(false);
			button.setTitle(&NSString::new());
			button.setEnabled(enabled);
		}
		Control::Input { field, target } => {
			target.sync_props(props);
			let custom = props.is_custom_drawn();
			field.setBezeled(!custom);
			field.setDrawsBackground(!custom);
			field.setFocusRingType(if custom {
				NSFocusRingType::None
			} else {
				NSFocusRingType::Default
			});
			field.setFont(Some(&system_font(props.text_style())));
			field.setTextColor(props.color.map(ns_color).as_deref());
			field.setAlignment(ns_alignment(props.text_style()));
			field.setBaseWritingDirection(match props.text_style().direction {
				leptoskit_yoga::Direction::Ltr => NSWritingDirection::LeftToRight,
				leptoskit_yoga::Direction::Rtl => NSWritingDirection::RightToLeft,
			});
			// A placeholder is the text color, faded; without a text color it
			// is the system's secondary label color, which adapts to the
			// appearance like the text does.
			let placeholder_color =
				props
					.color
					.map_or_else(NSColor::secondaryLabelColor, |color| {
						ns_color(color).colorWithAlphaComponent(
							PLACEHOLDER_OPACITY * f64::from(color.alpha()) / 255.0,
						)
					});
			let placeholder = props.placeholder.as_deref().map(|placeholder| {
				attributed_string(placeholder, props.text_style(), &placeholder_color)
			});
			field.setPlaceholderAttributedString(placeholder.as_deref());
			let value = props
				.value
				.as_deref()
				.or(props.text.as_deref())
				.unwrap_or_default();
			// Rewriting an unchanged value would move the caret while typing.
			if field.stringValue().to_string() != value {
				field.setStringValue(&NSString::from_str(value));
			}
			field.setEnabled(enabled);
		}
		Control::Image { image_view, source } => {
			if *source != props.source {
				let image = props.source.as_deref().and_then(load_image);
				image_view.setImage(image.as_deref());
				source.clone_from(&props.source);
			}
		}
	}
}

/// Loads an image from a `source`: a plain file path or a `file://` URL.
///
/// Other schemes (`https://`, `asset://`, …) are not loaded and leave the
/// image view empty, as does a file that is missing or not an image.
fn load_image(source: &str) -> Option<Retained<NSImage>> {
	let path = match source.split_once("://") {
		Some(("file", path)) => path,
		Some(_) => return None,
		None => source,
	};
	NSImage::initWithContentsOfFile(NSImage::alloc(), &NSString::from_str(path))
}

impl NativeBackend for AppKitBackend {
	fn set_dark_mode(&mut self, dark: bool) {
		let _events = self.sink.suppress();
		// SAFETY: AppKit exports immutable appearance-name strings.
		let name = unsafe {
			if dark {
				NSAppearanceNameDarkAqua
			} else {
				NSAppearanceNameAqua
			}
		};
		if self
			.appearance
			.as_ref()
			.is_some_and(|current| current.name().isEqualToString(name))
		{
			return;
		}
		let appearance =
			NSAppearance::appearanceNamed(name).expect("AppKit supplies Aqua appearances");
		for widget in self.widgets.iter().flatten() {
			widget.view.setAppearance(Some(&appearance));
		}
		self.appearance = Some(appearance);
	}

	fn apply(&mut self, mutations: &MutationList) {
		let _events = self.sink.suppress();
		// Detaching a focused field can synchronously submit/end its editor.
		// Unhook controls that are leaving before any hierarchy mutation.
		for mutation in mutations.mutations() {
			if let Mutation::Delete { id } = mutation {
				deactivate_control(&self.expect_widget(*id).control);
			}
		}

		// AppKit's convenience constructors autorelease; draining a pool per
		// batch keeps a long-running surface from accumulating them.
		autoreleasepool(|_| {
			self.applying = true;
			for mutation in mutations.mutations() {
				self.apply_one(mutation);
			}
			self.applying = false;
			let moves_views = mutations.mutations().iter().any(|mutation| {
				matches!(
					mutation,
					Mutation::UpdateLayout { .. }
						| Mutation::InsertChild { .. }
						| Mutation::RemoveChild { .. }
				)
			});
			if moves_views {
				self.refresh_scroll_documents();
			}
			self.refresh_radios();
			self.refresh_animations();
		});
	}

	fn create(&mut self, id: NodeId, kind: NativeKind, props: &NativeProps) {
		let mut widget = self.build(id, kind);
		apply_common(&widget.view, props, widget.kind);
		apply_control(&mut widget.control, props);
		if self.widgets.len() <= id.index() {
			self.widgets.resize_with(id.index() + 1, || None);
		}
		if kind == NativeKind::ScrollView {
			self.scroll_views.push(id);
		}
		self.widgets[id.index()] = Some(widget);
		self.refresh_controls_if_unbatched();
	}

	fn update_props(&mut self, id: NodeId, props: &NativeProps) {
		let widget = self.expect_widget_mut(id);
		apply_common(&widget.view, props, widget.kind);
		apply_control(&mut widget.control, props);
		let live = props.aria().effective_live(widget.kind);
		let announced = (live != leptoskit_native::Live::Off)
			.then(|| props.aria().live_text.clone())
			.flatten()
			.filter(|text| !text.is_empty());
		if announced != widget.announced {
			if let Some(text) = &announced {
				crate::accessibility::announce(&widget.view, text, live);
			}
			widget.announced = announced;
		}
		self.refresh_controls_if_unbatched();
	}

	fn update_layout(&mut self, id: NodeId, x: f32, y: f32, width: f32, height: f32) {
		let widget = self.expect_widget(id);
		let measured = matches!(widget.control, Control::Label(_) | Control::Button { .. });
		widget
			.view
			.setFrame(pixel_aligned(x, y, width, height, measured));
		sync_corner_curve(&widget.view, widget.kind);
	}

	fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
		let child_view = self.expect_widget(child).view.clone();
		let parent_widget = self.expect_widget(parent);
		let index = index.min(parent_widget.children.len());
		match parent_widget.children.get(index) {
			Some(&sibling) => {
				let sibling_view = self.expect_widget(sibling).view.clone();
				// Below the current occupant of the slot means just before it
				// in the subview order, which is drawing order.
				parent_widget.content().addSubview_positioned_relativeTo(
					&child_view,
					NSWindowOrderingMode::Below,
					Some(&sibling_view),
				);
			}
			None => parent_widget.content().addSubview(&child_view),
		}
		self.expect_widget_mut(parent).children.insert(index, child);
		self.refresh_controls_if_unbatched();
	}

	fn remove_child(&mut self, parent: NodeId, child: NodeId) {
		self.expect_widget(child).view.removeFromSuperview();
		self.expect_widget_mut(parent)
			.children
			.retain(|&existing| existing != child);
		self.refresh_controls_if_unbatched();
	}

	fn delete(&mut self, id: NodeId) {
		let Some(widget) = self.widgets.get_mut(id.index()).and_then(Option::take) else {
			panic!("mutation deletes {id:?}, which the AppKit backend does not hold");
		};
		// The renderer detaches before deleting; this only guards against a
		// view outliving its node inside a still-live parent.
		deactivate_control(&widget.control);
		widget.view.removeFromSuperview();
		if widget.kind == NativeKind::ScrollView {
			self.scroll_views.retain(|&existing| existing != id);
		}
		self.refresh_controls_if_unbatched();
	}

	fn text_measurer(&self) -> SharedTextMeasurer {
		Arc::clone(&self.measurer)
	}
}

/// Delivers a scroll view's offset changes to `target` as
/// [`NativeEvent::Scroll`]: its clip view posts a bounds notification
/// whenever the content moves.
fn observe_scrolling(scroll: &NSScrollView, target: &ActionTarget) {
	let clip = scroll.contentView();
	clip.setPostsBoundsChangedNotifications(true);
	// SAFETY: `boundsDidChange:` is implemented by `ActionTarget` and takes
	// the posted `NSNotification`; the name is AppKit's immutable constant;
	// `deactivate_control` removes the observer before the target can go.
	unsafe {
		NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
			target,
			sel!(boundsDidChange:),
			Some(NSViewBoundsDidChangeNotification),
			Some(&clip),
		);
	}
}

/// Invalidates weak actions before dropping their controls, and always stops
/// native timers even when a caller retains a detached indicator view.
fn deactivate_control(control: &Control) {
	match control {
		Control::Button { button, target }
		| Control::Pressable { button, target }
		| Control::Toggle { button, target, .. } => {
			target.invalidate();
			// SAFETY: clearing a target/action leaves a retained detached
			// control inert.
			unsafe {
				button.setTarget(None);
				button.setAction(None);
			}
		}
		Control::Input { field, target } => {
			target.invalidate();
			// SAFETY: the owned field no longer has a live node or delegate.
			unsafe {
				field.setTarget(None);
				field.setAction(None);
				field.setDelegate(None);
			}
		}
		Control::Switch { switch, target } => {
			target.invalidate();
			// SAFETY: clearing a target/action leaves a retained detached
			// control inert.
			unsafe {
				switch.setTarget(None);
				switch.setAction(None);
			}
		}
		Control::Slider { slider, target } => {
			target.invalidate();
			// SAFETY: clearing a target/action leaves a retained detached
			// control inert.
			unsafe {
				slider.setTarget(None);
				slider.setAction(None);
			}
		}
		Control::Scroll { target, .. } => {
			target.invalidate();
			// SAFETY: the target is a live object registered by
			// `observe_scrolling`; removing an observer is idempotent.
			unsafe {
				NSNotificationCenter::defaultCenter().removeObserver(target);
			}
		}
		Control::Progress { indicator, .. } => {
			// SAFETY: stopAnimation: accepts an optional sender and is
			// idempotent.
			unsafe {
				indicator.stopAnimation(None);
			}
		}
		_ => {}
	}
}

impl Drop for AppKitBackend {
	fn drop(&mut self) {
		for widget in self.widgets.iter().flatten() {
			deactivate_control(&widget.control);
		}
	}
}
