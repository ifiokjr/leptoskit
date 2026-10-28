//! UIKit mutation application and native widget ownership.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use leptoskit_native::Color;
use leptoskit_native::Mutation;
use leptoskit_native::MutationList;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::Progress;
use leptoskit_native::Role;
use leptoskit_native::SharedTextMeasurer;
use leptoskit_yoga::NodeId;
use objc2::MainThreadMarker;
use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2::rc::autoreleasepool;
use objc2::runtime::ProtocolObject;
use objc2::sel;
use objc2_core_foundation::CGPoint;
use objc2_core_foundation::CGRect;
use objc2_core_foundation::CGSize;
use objc2_foundation::NSNumber;
use objc2_foundation::NSString;
use objc2_foundation::NSURL;
use objc2_quartz_core::CABasicAnimation;
use objc2_quartz_core::CAMediaTiming;
use objc2_ui_kit::NSObjectUIAccessibility;
use objc2_ui_kit::UIAccessibilityTraitButton;
use objc2_ui_kit::UIAccessibilityTraitNotEnabled;
use objc2_ui_kit::UIAccessibilityTraitSelected;
use objc2_ui_kit::UIActivityIndicatorView;
use objc2_ui_kit::UIActivityIndicatorViewStyle;
use objc2_ui_kit::UIButton;
use objc2_ui_kit::UIButtonType;
use objc2_ui_kit::UIColor;
use objc2_ui_kit::UIControl;
use objc2_ui_kit::UIControlEvents;
use objc2_ui_kit::UIControlState;
use objc2_ui_kit::UIFont;
use objc2_ui_kit::UIImage;
use objc2_ui_kit::UIImageView;
use objc2_ui_kit::UILabel;
use objc2_ui_kit::UIProgressView;
use objc2_ui_kit::UIProgressViewStyle;
use objc2_ui_kit::UIScrollView;
use objc2_ui_kit::UIScrollViewContentInsetAdjustmentBehavior;
use objc2_ui_kit::UISlider;
use objc2_ui_kit::UISwitch;
use objc2_ui_kit::UITextField;
use objc2_ui_kit::UITextInput;
use objc2_ui_kit::UITextInputTraits;
use objc2_ui_kit::UIUserInterfaceStyle;
use objc2_ui_kit::UIView;
use objc2_ui_kit::UIViewContentMode;

use crate::events::ActionTarget;
use crate::events::ContentButton;
use crate::events::ControlledSlider;
use crate::events::EventListener;
use crate::events::EventSink;
use crate::events::RadioButton;
use crate::events::RadioMember;
use crate::events::input_value;
use crate::events::slider_coordinate;
use crate::events::visible;
use crate::measure::UIKitTextMeasurer;
use crate::measure::font_weight;
use crate::text::alignment;
use crate::text::attributed;
use crate::vector::VectorLayers;

/// Renders an existing shared mutation stream as UIKit views and controls.
///
/// Create, access, commit, and drop this backend on the main thread. Mount its
/// root view in an application-owned view controller, or use
/// [`Self::open_window`]. Programmatic changes suppress delegate callbacks.
/// User proposals restore the latest committed value after the listener
/// returns, even after replacement.
pub struct UIKitBackend {
	mtm: MainThreadMarker,
	widgets: Vec<Option<Widget>>,
	sink: Rc<EventSink>,
	dark: Option<bool>,
	applying: bool,
	animations_active: Cell<bool>,
}

impl std::fmt::Debug for UIKitBackend {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("UIKitBackend")
			.field("widgets", &self.widgets.iter().flatten().count())
			.finish_non_exhaustive()
	}
}

struct Widget {
	id: NodeId,
	kind: NativeKind,
	view: Retained<UIView>,
	control: Control,
	props: NativeProps,
	children: Vec<NodeId>,
}

enum Control {
	Plain,
	Label(Retained<UILabel>),
	Button {
		button: Retained<UIButton>,
		target: Retained<ActionTarget>,
	},
	Switch {
		switch: Retained<UISwitch>,
		target: Retained<ActionTarget>,
	},
	Slider {
		slider: Retained<UISlider>,
		target: Retained<ActionTarget>,
	},
	Input {
		field: Retained<UITextField>,
		target: Retained<ActionTarget>,
	},
	Image {
		image: Retained<UIImageView>,
		source: Option<String>,
	},
	Scroll {
		scroll: Retained<UIScrollView>,
		target: Retained<ActionTarget>,
	},
	Progress {
		progress: Retained<UIProgressView>,
		fill: Retained<UIView>,
		animating: Cell<bool>,
	},
	Activity(Retained<UIActivityIndicatorView>),
	Vector(VectorLayers),
}

impl Widget {
	fn target(&self) -> Option<&ActionTarget> {
		match &self.control {
			Control::Button { target, .. }
			| Control::Switch { target, .. }
			| Control::Slider { target, .. }
			| Control::Input { target, .. }
			| Control::Scroll { target, .. } => Some(target),
			_ => None,
		}
	}

	fn deactivate(&self) {
		if let Some(target) = self.target() {
			target.invalidate();
		}
		if let Some(control) = self.view.downcast_ref::<UIControl>() {
			// SAFETY: Removing all registered target/actions leaves retained,
			// detached controls inert before their node and target disappear.
			unsafe {
				control.removeTarget_action_forControlEvents(
					None,
					None,
					UIControlEvents::AllEvents,
				);
			}
		}
		match &self.control {
			Control::Switch { switch, .. } => {
				// SAFETY: The switch's owned target is about to be invalidated.
				unsafe {
					switch.removeTarget_action_forControlEvents(
						None,
						None,
						UIControlEvents::AllEvents,
					);
				}
			}
			Control::Input { field, .. } => field.setDelegate(None),
			Control::Scroll { scroll, .. } => {
				// SAFETY: Clearing the weak delegate during teardown is valid.
				unsafe {
					scroll.setDelegate(None);
				}
			}
			Control::Activity(indicator) => indicator.stopAnimating(),
			Control::Progress { fill, .. } => fill.layer().removeAllAnimations(),
			_ => {}
		}
	}
}

impl UIKitBackend {
	/// Creates a backend which uses independent Core Text measurement.
	pub fn new(mtm: MainThreadMarker) -> Self {
		Self {
			mtm,
			widgets: Vec::new(),
			sink: Rc::new(EventSink::default()),
			dark: None,
			applying: false,
			animations_active: Cell::new(true),
		}
	}

	/// The proof that this backend belongs to the UI thread.
	pub fn main_thread_marker(&self) -> MainThreadMarker {
		self.mtm
	}

	pub(crate) fn suppress_events(&self) -> crate::events::Suppression {
		self.sink.suppress()
	}

	pub(crate) fn pause_animations(&self) {
		self.animations_active.set(false);
		for widget in self.widgets.iter().flatten() {
			match &widget.control {
				Control::Activity(indicator) => indicator.stopAnimating(),
				Control::Progress {
					fill, animating, ..
				} => {
					fill.layer().removeAllAnimations();
					animating.set(false);
				}
				_ => {}
			}
		}
	}

	pub(crate) fn resume_animations(&self) {
		self.animations_active.set(true);
	}

	/// Replaces the listener. Callbacks may synchronously rebuild the surface.
	pub fn set_event_listener(&mut self, listener: Option<EventListener>) {
		self.sink.set_listener(listener);
	}

	/// A live node's actual UIKit view. Retaining it does not retain its node.
	pub fn view(&self, id: NodeId) -> Option<&UIView> {
		self.widgets
			.get(id.index())
			.and_then(Option::as_ref)
			.filter(|widget| widget.id == id)
			.map(|widget| &*widget.view)
	}

	fn widget(&self, id: NodeId) -> &Widget {
		self.widgets
			.get(id.index())
			.and_then(Option::as_ref)
			.filter(|widget| widget.id == id)
			.expect("mutation references a live UIKit node")
	}

	fn widget_mut(&mut self, id: NodeId) -> &mut Widget {
		self.widgets
			.get_mut(id.index())
			.and_then(Option::as_mut)
			.filter(|widget| widget.id == id)
			.expect("mutation references a live UIKit node")
	}

	fn build(&self, id: NodeId, kind: NativeKind) -> Widget {
		let frame = CGRect::ZERO;
		let target = || ActionTarget::new(self.mtm, id, kind, Rc::clone(&self.sink));
		let (view, control) = match kind {
			NativeKind::Text | NativeKind::TextRun => {
				let label = UILabel::initWithFrame(UILabel::alloc(self.mtm), frame);
				label.setNumberOfLines(0);
				label.setLineBreakMode(objc2_ui_kit::NSLineBreakMode::ByWordWrapping);
				(label.clone().into_super(), Control::Label(label))
			}
			NativeKind::Button
			| NativeKind::Checkbox
			| NativeKind::Radio
			| NativeKind::Pressable => {
				let target = target();
				let button = self.build_button(kind, &target);
				(
					button.clone().into_super().into_super(),
					Control::Button { button, target },
				)
			}
			NativeKind::Switch => {
				let view = UIView::initWithFrame(UIView::alloc(self.mtm), frame);
				let switch = UISwitch::initWithFrame(UISwitch::alloc(self.mtm), frame);
				let target = target();
				connect(
					&switch,
					&target,
					sel!(activate:),
					UIControlEvents::ValueChanged,
				);
				view.addSubview(&switch);
				(view, Control::Switch { switch, target })
			}
			NativeKind::Slider => {
				let target = target();
				let slider = ControlledSlider::new(self.mtm, &target).into_super();
				slider.setMinimumValue(0.0);
				slider.setMaximumValue(1.0);
				connect(
					&slider,
					&target,
					sel!(activate:),
					UIControlEvents::ValueChanged,
				);
				(
					slider.clone().into_super().into_super(),
					Control::Slider { slider, target },
				)
			}
			NativeKind::TextInput | NativeKind::PasswordInput => {
				let field = UITextField::initWithFrame(UITextField::alloc(self.mtm), frame);
				field.setSecureTextEntry(kind == NativeKind::PasswordInput);
				let target = target();
				field.setDelegate(Some(ProtocolObject::from_ref(&*target)));
				connect(
					&field,
					&target,
					sel!(edit:),
					UIControlEvents::EditingChanged,
				);
				(
					field.clone().into_super().into_super(),
					Control::Input { field, target },
				)
			}
			NativeKind::Image => {
				let image = UIImageView::initWithFrame(UIImageView::alloc(self.mtm), frame);
				image.setContentMode(UIViewContentMode::ScaleAspectFit);
				(
					image.clone().into_super(),
					Control::Image {
						image,
						source: None,
					},
				)
			}
			NativeKind::ScrollView => {
				let scroll = UIScrollView::initWithFrame(UIScrollView::alloc(self.mtm), frame);
				scroll.setContentInsetAdjustmentBehavior(
					UIScrollViewContentInsetAdjustmentBehavior::Never,
				);
				let target = target();
				// SAFETY: Widget owns the delegate while this scroll view
				// lives.
				unsafe {
					scroll.setDelegate(Some(ProtocolObject::from_ref(&*target)));
				}
				(
					scroll.clone().into_super(),
					Control::Scroll { scroll, target },
				)
			}
			NativeKind::LinearProgressIndicator => {
				let view = UIView::initWithFrame(UIView::alloc(self.mtm), frame);
				let progress = UIProgressView::initWithProgressViewStyle(
					UIProgressView::alloc(self.mtm),
					UIProgressViewStyle::Default,
				);
				let fill = UIView::initWithFrame(UIView::alloc(self.mtm), frame);
				view.addSubview(&progress);
				view.addSubview(&fill);
				progress.setIsAccessibilityElement(false, self.mtm);
				view.setClipsToBounds(true);
				(
					view,
					Control::Progress {
						progress,
						fill,
						animating: Cell::new(false),
					},
				)
			}
			NativeKind::ActivityIndicator => {
				let indicator = UIActivityIndicatorView::initWithActivityIndicatorStyle(
					UIActivityIndicatorView::alloc(self.mtm),
					UIActivityIndicatorViewStyle::Medium,
				);
				indicator.setHidesWhenStopped(true);
				(indicator.clone().into_super(), Control::Activity(indicator))
			}
			NativeKind::Vector => {
				(
					UIView::initWithFrame(UIView::alloc(self.mtm), frame),
					Control::Vector(VectorLayers::default()),
				)
			}
			NativeKind::View | NativeKind::Custom(_) => {
				(
					UIView::initWithFrame(UIView::alloc(self.mtm), frame),
					Control::Plain,
				)
			}
		};
		if let Some(dark) = self.dark {
			view.setOverrideUserInterfaceStyle(appearance(dark));
		}
		Widget {
			id,
			kind,
			view,
			control,
			props: NativeProps::default(),
			children: Vec::new(),
		}
	}

	/// The button class for a pressable, radio, checkbox, or plain button,
	/// already wired to its action target.
	fn build_button(&self, kind: NativeKind, target: &ActionTarget) -> Retained<UIButton> {
		let button = if kind == NativeKind::Pressable {
			ContentButton::new(self.mtm).into_super()
		} else if kind == NativeKind::Radio {
			RadioButton::new(self.mtm, target).into_super()
		} else {
			UIButton::buttonWithType(
				if kind == NativeKind::Button {
					UIButtonType::System
				} else {
					UIButtonType::Custom
				},
				self.mtm,
			)
		};
		connect(
			&button,
			target,
			sel!(activate:),
			UIControlEvents::PrimaryActionTriggered,
		);
		if matches!(kind, NativeKind::Button | NativeKind::Pressable) {
			// These are the events on which UIKit sets and clears the
			// button's highlight, so `active:` styles follow it.
			connect(
				&button,
				target,
				sel!(pressIn:),
				UIControlEvents::TouchDown | UIControlEvents::TouchDragEnter,
			);
			connect(
				&button,
				target,
				sel!(pressOut:),
				UIControlEvents::TouchDragExit
					| UIControlEvents::TouchUpInside
					| UIControlEvents::TouchUpOutside
					| UIControlEvents::TouchCancel,
			);
		}
		button
	}

	fn refresh(&mut self) {
		for widget in self.widgets.iter().flatten() {
			if let Control::Scroll { scroll, .. } = &widget.control {
				let mut size = widget.view.bounds().size;
				for child in &widget.children {
					let frame = self.widget(*child).view.frame();
					size.width = size.width.max(frame.origin.x + frame.size.width);
					size.height = size.height.max(frame.origin.y + frame.size.height);
				}
				scroll.setContentSize(size);
			}
		}
		let children: std::collections::HashSet<_> = self
			.widgets
			.iter()
			.flatten()
			.flat_map(|widget| widget.children.iter().copied())
			.collect();
		let mut pending: Vec<_> = self
			.widgets
			.iter()
			.flatten()
			.filter(|widget| !children.contains(&widget.id))
			.map(|widget| widget.id)
			.collect();
		let mut radios = Vec::new();
		while let Some(node) = pending.pop() {
			let widget = self.widget(node);
			pending.extend(widget.children.iter().rev().copied());
			if widget.kind == NativeKind::Radio {
				if let Control::Button { button, target } = &widget.control {
					radios.push(RadioMember {
						button: objc2::rc::Weak::new(button),
						target: objc2::rc::Weak::new(target),
					});
				}
			}
		}
		self.sink.set_radios(radios);
		for widget in self.widgets.iter_mut().flatten() {
			let available = visible(&widget.view) && self.animations_active.get();
			if let Some(control) = widget.view.downcast_ref::<UIControl>() {
				let mut enabled = widget.props.enabled.unwrap_or(true);
				let mut parent = widget.view.superview();
				while let Some(view) = parent {
					enabled &= view.isUserInteractionEnabled();
					parent = view.superview();
				}
				control.setEnabled(enabled);
			}
			let mut parent = widget.view.superview();
			while let Some(view) = parent {
				if view.downcast_ref::<ContentButton>().is_some() {
					widget.view.setUserInteractionEnabled(false);
					widget.view.setAccessibilityElementsHidden(true, self.mtm);
					widget.view.setIsAccessibilityElement(false, self.mtm);
					break;
				}
				parent = view.superview();
			}
			match &mut widget.control {
				Control::Activity(indicator) => {
					let mut requested = self.animations_active.get()
						&& widget.props.running.unwrap_or(true)
						&& !widget.props.hidden.unwrap_or(false)
						&& widget.props.opacity.unwrap_or(1.0) > 0.01;
					let mut parent = indicator.superview();
					while let Some(view) = parent {
						requested &= visible(&view);
						parent = view.superview();
					}
					if requested {
						indicator.startAnimating();
					} else {
						indicator.stopAnimating();
					}
					indicator.setAccessibilityElementsHidden(!requested, self.mtm);
				}
				Control::Switch { switch, .. } => {
					let mut enabled = widget.props.enabled.unwrap_or(true);
					let mut parent = switch.superview();
					while let Some(view) = parent {
						enabled &= view.isUserInteractionEnabled();
						parent = view.superview();
					}
					switch.setEnabled(enabled);
				}
				Control::Progress {
					progress,
					fill,
					animating,
				} => {
					let indeterminate = matches!(
						widget.props.progress.unwrap_or_default(),
						Progress::Indeterminate
					);
					progress.setHidden(indeterminate);
					fill.setHidden(!indeterminate);
					if available && indeterminate && !animating.get() {
						let animation = CABasicAnimation::animationWithKeyPath(Some(
							&NSString::from_str("transform.translation.x"),
						));
						animation.setDuration(1.2);
						animation.setRepeatCount(f32::INFINITY);
						// SAFETY: A translation-x animation expects NSNumber
						// values.
						unsafe {
							animation.setFromValue(Some(&NSNumber::new_f64(0.0)));
							animation.setToValue(Some(&NSNumber::new_f64(
								widget.view.bounds().size.width * 1.4,
							)));
							fill.layer().addAnimation_forKey(
								&animation,
								Some(&NSString::from_str("leptoskit.indeterminate")),
							);
						}
						animating.set(true);
					} else if (!available || !indeterminate) && animating.get() {
						fill.layer().removeAllAnimations();
						animating.set(false);
					}
				}
				_ => {}
			}
		}
	}

	fn refresh_unbatched(&mut self) {
		if !self.applying {
			self.refresh();
		}
	}
}

fn connect(
	control: &UIControl,
	target: &ActionTarget,
	selector: objc2::runtime::Sel,
	event: UIControlEvents,
) {
	// SAFETY: The selector names an ActionTarget method with this UIControl
	// sender. The owning Widget retains the target until registration is
	// removed.
	unsafe {
		control.addTarget_action_forControlEvents(Some(target), selector, event);
	}
}

fn appearance(dark: bool) -> UIUserInterfaceStyle {
	if dark {
		UIUserInterfaceStyle::Dark
	} else {
		UIUserInterfaceStyle::Light
	}
}

fn color(color: Color) -> Retained<UIColor> {
	UIColor::colorWithRed_green_blue_alpha(
		f64::from(color.red()) / 255.0,
		f64::from(color.green()) / 255.0,
		f64::from(color.blue()) / 255.0,
		f64::from(color.alpha()) / 255.0,
	)
}

fn sync_corner_radius(view: &UIView, requested: Option<f32>) {
	let size = view.bounds().size;
	view.layer().setCornerRadius(
		f64::from(requested.unwrap_or(0.0))
			.max(0.0)
			.min(size.width / 2.0)
			.min(size.height / 2.0),
	);
}

fn apply_props(widget: &mut Widget, props: &NativeProps, mtm: MainThreadMarker) {
	let view = &widget.view;
	let foreground = props.color.map_or_else(UIColor::labelColor, color);
	let enabled = props.enabled.unwrap_or(true);
	view.setOpaque(false);
	view.setBackgroundColor(Some(
		&props.background.map_or_else(UIColor::clearColor, color),
	));
	view.setHidden(props.hidden.unwrap_or(false));
	view.setAlpha(f64::from(props.opacity.unwrap_or(1.0)));
	view.setUserInteractionEnabled(enabled);
	view.setSemanticContentAttribute(
		if props.text_style().direction == leptoskit_yoga::Direction::Rtl {
			objc2_ui_kit::UISemanticContentAttribute::ForceRightToLeft
		} else {
			objc2_ui_kit::UISemanticContentAttribute::ForceLeftToRight
		},
	);
	view.setClipsToBounds(
		props.clip.unwrap_or(false)
			|| props.corner_radius.unwrap_or(0.0) > 0.0
			|| matches!(
				widget.kind,
				NativeKind::LinearProgressIndicator | NativeKind::ScrollView
			),
	);
	sync_corner_radius(view, props.corner_radius);
	view.layer()
		.setBorderWidth(f64::from(props.border_width.unwrap_or(0.0)));
	// SAFETY: UIColor supplies a valid retained CGColor for Core Animation.
	unsafe {
		view.layer().setBorderColor(
			props
				.border_color
				.map(color)
				.map(|color| color.CGColor())
				.as_deref(),
		);
		view.setTintColor(Some(&foreground));
	}
	view.setAccessibilityLabel(
		props
			.accessibility_label
			.as_ref()
			.map(|text| NSString::from_str(text))
			.as_deref(),
		mtm,
	);
	crate::accessibility::apply(view, props, widget.kind, mtm);
	if let Some(control) = view.downcast_ref::<UIControl>() {
		control.setEnabled(enabled);
	}
	if let Some(target) = widget.target() {
		target.sync(props);
	}
	let font = UIFont::systemFontOfSize_weight(
		f64::from(props.text_style().font_size),
		font_weight(props.text_style().font_weight),
	);
	match &mut widget.control {
		Control::Plain => {
			view.setIsAccessibilityElement(crate::accessibility::is_element(props), mtm);
		}
		Control::Label(label) => {
			label.setAttributedText(Some(&attributed(
				props.text.as_deref().unwrap_or_default(),
				props.text_style(),
				&foreground,
			)));
			label.setTextAlignment(alignment(props.text_style()));
		}
		Control::Button { button, .. } => {
			apply_button_props(button, widget.kind, props, &foreground, &font, mtm);
		}
		Control::Switch { switch, .. } => {
			switch.setOn(props.checked.unwrap_or(false));
			switch.setEnabled(enabled);
			switch.setOnTintColor(props.color.map(color).as_deref());
			switch.setAccessibilityLabel(
				props
					.accessibility_label
					.as_ref()
					.map(|text| NSString::from_str(text))
					.as_deref(),
				mtm,
			);
			view.setIsAccessibilityElement(false, mtm);
		}
		Control::Slider { slider, .. } => {
			let state = props.slider.unwrap_or_default();
			slider.setValue(slider_coordinate(state));
			let announced = props
				.aria()
				.value_text
				.as_deref()
				.map_or_else(|| state.value().to_string(), str::to_owned);
			slider.setAccessibilityValue(Some(&NSString::from_str(&announced)), mtm);
		}
		Control::Input { field, .. } => apply_input_props(field, props, &foreground, &font),
		Control::Image { image, source } => {
			if source != &props.source {
				let loaded = props.source.as_deref().and_then(load_image);
				image.setImage(loaded.as_deref());
				source.clone_from(&props.source);
			}
			view.setIsAccessibilityElement(crate::accessibility::is_element(props), mtm);
		}
		Control::Vector(layers) => {
			if widget.props.drawing != props.drawing {
				layers.draw(view, props.drawing.as_ref());
			}
			// A named drawing reads as one image; an unnamed one is decorative.
			view.setIsAccessibilityElement(crate::accessibility::is_element(props), mtm);
			view.setAccessibilityTraits(
				crate::accessibility::traits(props.aria().role.unwrap_or(Role::Img)),
				mtm,
			);
		}
		Control::Scroll { scroll, .. } => {
			scroll.setScrollEnabled(enabled);
		}
		Control::Progress { progress, fill, .. } => {
			apply_progress_props(view, progress, fill, props, &foreground, mtm);
		}
		Control::Activity(indicator) => {
			// SAFETY: The owned UIKit color has the expected UIColor type.
			unsafe {
				indicator.setColor(Some(&foreground));
			}
			indicator.setIsAccessibilityElement(true, mtm);
			if props.accessibility_label.is_none() {
				indicator.setAccessibilityLabel(Some(&NSString::from_str("Loading")), mtm);
			}
		}
	}
	let live = props.aria().effective_live(widget.kind);
	if live != leptoskit_native::Live::Off
		&& let Some(text) = props
			.aria()
			.live_text
			.as_deref()
			.filter(|text| !text.is_empty())
		&& widget.props.aria().live_text.as_deref() != Some(text)
	{
		crate::accessibility::announce(text);
	}
	widget.props.clone_from(props);
}

fn apply_button_props(
	button: &UIButton,
	kind: NativeKind,
	props: &NativeProps,
	foreground: &UIColor,
	font: &UIFont,
	mtm: MainThreadMarker,
) {
	let toggle = matches!(kind, NativeKind::Checkbox | NativeKind::Radio);
	let enabled = props.enabled.unwrap_or(true);
	button.setSelected(props.checked.unwrap_or(false));
	button.setTitle_forState(
		props
			.text
			.as_ref()
			.filter(|_| kind != NativeKind::Pressable)
			.map(|text| NSString::from_str(text))
			.as_deref(),
		UIControlState::Normal,
	);
	button.setTitleColor_forState(Some(foreground), UIControlState::Normal);
	if let Some(label) = button.titleLabel() {
		unsafe {
			label.setFont(Some(font));
		}
	}
	if toggle {
		let (off, on) = if kind == NativeKind::Radio {
			("circle", "largecircle.fill.circle")
		} else {
			("square", "checkmark.square.fill")
		};
		button.setImage_forState(
			UIImage::systemImageNamed(&NSString::from_str(off)).as_deref(),
			UIControlState::Normal,
		);
		button.setImage_forState(
			UIImage::systemImageNamed(&NSString::from_str(on)).as_deref(),
			UIControlState::Selected,
		);
		// SAFETY: UIKit exports immutable accessibility bit flags.
		let traits = unsafe {
			UIAccessibilityTraitButton
				| if props.checked.unwrap_or(false) {
					UIAccessibilityTraitSelected
				} else {
					0
				}
				| if enabled {
					0
				} else {
					UIAccessibilityTraitNotEnabled
				}
		};
		button.setAccessibilityTraits(traits, mtm);
		button.setAccessibilityValue(
			Some(&NSString::from_str(if props.checked.unwrap_or(false) {
				"1"
			} else {
				"0"
			})),
			mtm,
		);
	}
	if kind == NativeKind::Pressable {
		button.setIsAccessibilityElement(true, mtm);
	}
}

fn apply_input_props(
	field: &UITextField,
	props: &NativeProps,
	foreground: &UIColor,
	font: &UIFont,
) {
	let value = input_value(props);
	if field
		.text()
		.map(|text| text.to_string())
		.unwrap_or_default()
		!= value
	{
		field.setText(Some(&NSString::from_str(value)));
	}
	field.setPlaceholder(
		props
			.placeholder
			.as_ref()
			.map(|text| NSString::from_str(text))
			.as_deref(),
	);
	field.setFont(Some(font));
	field.setTextColor(Some(foreground));
	field.setTextAlignment(alignment(props.text_style()));
	let beginning = field.beginningOfDocument();
	let end = field.endOfDocument();
	if let Some(range) = field.textRangeFromPosition_toPosition(&beginning, &end) {
		field.setBaseWritingDirection_forRange(
			if props.text_style().direction == leptoskit_yoga::Direction::Rtl {
				objc2_ui_kit::NSWritingDirection::RightToLeft
			} else {
				objc2_ui_kit::NSWritingDirection::LeftToRight
			},
			&range,
		);
	}
}

/// Loads an image from a bundle `resource://` name, a local `file:` URL, or a
/// plain path. Other URL schemes, and file URLs naming a remote host, load
/// nothing.
fn load_image(source: &str) -> Option<Retained<UIImage>> {
	if let Some(name) = source.strip_prefix("resource://") {
		return UIImage::imageNamed(&NSString::from_str(name));
	}
	if source.starts_with("file:") {
		let url = NSURL::URLWithString(&NSString::from_str(source))?;
		if !url.isFileURL()
			|| url.host().is_some_and(|host| {
				!host.is_empty() && !host.isEqualToString(&NSString::from_str("localhost"))
			}) {
			return None;
		}
		let path = url.path()?;
		return UIImage::imageWithContentsOfFile(&path);
	}
	if source.contains("://") {
		return None;
	}
	UIImage::imageWithContentsOfFile(&NSString::from_str(source))
}

fn apply_progress_props(
	view: &UIView,
	progress: &UIProgressView,
	fill: &UIView,
	props: &NativeProps,
	foreground: &UIColor,
	mtm: MainThreadMarker,
) {
	progress.setProgressTintColor(Some(foreground));
	progress.setTrackTintColor(props.background.map(color).as_deref());
	fill.setBackgroundColor(Some(foreground));
	if let Progress::Determinate(value) = props.progress.unwrap_or_default() {
		progress.setProgress(value.get() as f32);
		view.setAccessibilityValue(
			Some(&NSString::from_str(&format!(
				"{}%",
				(value.get() * 100.0).round()
			))),
			mtm,
		);
	} else {
		view.setAccessibilityValue(Some(&NSString::from_str("Loading")), mtm);
	}
	view.setIsAccessibilityElement(true, mtm);
}

impl NativeBackend for UIKitBackend {
	fn apply(&mut self, mutations: &MutationList) {
		let _events = self.sink.suppress();
		for mutation in mutations.mutations() {
			if let Mutation::Delete { id } = mutation {
				self.widget(*id).deactivate();
			}
		}
		autoreleasepool(|_| {
			self.applying = true;
			for mutation in mutations.mutations() {
				self.apply_one(mutation);
			}
			self.applying = false;
			self.refresh();
		});
	}

	fn create(&mut self, id: NodeId, kind: NativeKind, props: &NativeProps) {
		let _events = self.sink.suppress();
		let mut widget = self.build(id, kind);
		apply_props(&mut widget, props, self.mtm);
		if self.widgets.len() <= id.index() {
			self.widgets.resize_with(id.index() + 1, || None);
		}
		self.widgets[id.index()] = Some(widget);
		self.refresh_unbatched();
	}

	fn update_props(&mut self, id: NodeId, props: &NativeProps) {
		let _events = self.sink.suppress();
		let mtm = self.mtm;
		apply_props(self.widget_mut(id), props, mtm);
		self.refresh_unbatched();
	}

	fn update_layout(&mut self, id: NodeId, x: f32, y: f32, width: f32, height: f32) {
		let _events = self.sink.suppress();
		let widget = self.widget_mut(id);
		let size = CGSize::new(f64::from(width), f64::from(height));
		let resized = widget.view.frame().size != size;
		widget
			.view
			.setFrame(CGRect::new(CGPoint::new(f64::from(x), f64::from(y)), size));
		sync_corner_radius(&widget.view, widget.props.corner_radius);
		// Only a new size changes where the drawing maps; moving the frame
		// carries its layers along.
		if resized {
			if let Control::Vector(layers) = &mut widget.control {
				layers.draw(&widget.view, widget.props.drawing.as_ref());
			}
		}
		if let Control::Switch { switch, .. } = &widget.control {
			let native = switch.bounds().size;
			let scale = if native.width > 0.0 && native.height > 0.0 {
				(f64::from(width) / native.width).min(f64::from(height) / native.height)
			} else {
				1.0
			};
			switch.setTransform(objc2_core_foundation::CGAffineTransform {
				a: scale,
				b: 0.0,
				c: 0.0,
				d: scale,
				tx: 0.0,
				ty: 0.0,
			});
			switch.setCenter(CGPoint::new(
				f64::from(width) / 2.0,
				f64::from(height) / 2.0,
			));
		}
		if let Control::Progress {
			progress,
			fill,
			animating,
		} = &mut widget.control
		{
			progress.setTransform(objc2_core_foundation::CGAffineTransform {
				a: 1.0,
				b: 0.0,
				c: 0.0,
				d: 1.0,
				tx: 0.0,
				ty: 0.0,
			});
			progress.setFrame(widget.view.bounds());
			let native_height = progress.bounds().size.height;
			if native_height > 0.0 {
				progress.setTransform(objc2_core_foundation::CGAffineTransform {
					a: 1.0,
					b: 0.0,
					c: 0.0,
					d: f64::from(height) / native_height,
					tx: 0.0,
					ty: 0.0,
				});
				progress.setCenter(CGPoint::new(
					f64::from(width) / 2.0,
					f64::from(height) / 2.0,
				));
			}
			fill.setFrame(CGRect::new(
				CGPoint::new(-f64::from(width) * 0.3, 0.0),
				CGSize::new(f64::from(width) * 0.3, f64::from(height)),
			));
			fill.layer().removeAllAnimations();
			animating.set(false);
		}
		self.refresh_unbatched();
	}

	fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
		let _events = self.sink.suppress();
		let view = self.widget(child).view.clone();
		let parent_widget = self.widget(parent);
		let index = index.min(parent_widget.children.len());
		parent_widget
			.view
			.insertSubview_atIndex(&view, index as isize);
		if parent_widget.kind == NativeKind::Pressable {
			view.setUserInteractionEnabled(false);
			view.setAccessibilityElementsHidden(true, self.mtm);
			view.setIsAccessibilityElement(false, self.mtm);
		}
		self.widget_mut(parent).children.insert(index, child);
		self.refresh_unbatched();
	}

	fn remove_child(&mut self, parent: NodeId, child: NodeId) {
		let _events = self.sink.suppress();
		self.widget(child).view.removeFromSuperview();
		self.widget_mut(parent)
			.children
			.retain(|node| *node != child);
		self.refresh_unbatched();
	}

	fn delete(&mut self, id: NodeId) {
		let _events = self.sink.suppress();
		let widget = self
			.widgets
			.get_mut(id.index())
			.and_then(Option::take)
			.filter(|widget| widget.id == id)
			.expect("mutation deletes a live UIKit node");
		widget.deactivate();
		widget.view.removeFromSuperview();
		self.refresh_unbatched();
	}

	fn text_measurer(&self) -> SharedTextMeasurer {
		Arc::new(UIKitTextMeasurer::new())
	}

	fn set_dark_mode(&mut self, dark: bool) {
		let _events = self.sink.suppress();
		self.dark = Some(dark);
		for widget in self.widgets.iter().flatten() {
			widget.view.setOverrideUserInterfaceStyle(appearance(dark));
		}
	}
}

impl Drop for UIKitBackend {
	fn drop(&mut self) {
		let _events = self.sink.suppress();
		for widget in self.widgets.iter().flatten() {
			widget.deactivate();
		}
	}
}
