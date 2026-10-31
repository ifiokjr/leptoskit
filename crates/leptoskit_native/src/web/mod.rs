//! Render shared leptoskit elements as genuine Leptos DOM views.
//!
//! Pass [`render`] the same [`Element`] used with a native surface. Wrap the
//! call in a reactive closure to rebuild when Leptos signals change. Leptos
//! reconciles existing DOM nodes, preserving focus and input selection.
//!
//! Styles have deterministic, content-derived scopes, so server rendering and
//! hydration agree and independently rendered trees cannot affect one another's
//! class precedence. Browser media queries update responsive classes directly.
//!
//! Accessibility properties render as their ARIA attributes, and structural
//! roles become semantic elements: headings, lists, and landmarks use their
//! HTML elements rather than a `div` with a `role`. Containers that declare a
//! control role join the tab order and activate from Enter and Space.

use std::collections::BTreeMap;
use std::sync::Arc;

use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use leptos::tachys::html::attribute::Attribute;
use leptos::tachys::html::attribute::custom::custom_attribute;
use leptos::tachys::html::element::custom;
use leptos::tachys::html::event::on;
use sha2::Digest;
use sha2::Sha256;
use wasm_bindgen::JsCast;

use crate::Element;
use crate::EventOutcome;
use crate::NativeEvent;
use crate::NativeKind;
use crate::NativeProps;
use crate::Progress;
use crate::element::HandlerEntry;

mod controls;
#[cfg(feature = "rive")]
mod rive;
mod semantics;
mod styles;
mod vector;

#[cfg(all(feature = "rive", target_arch = "wasm32", target_os = "unknown"))]
pub use rive::provide_rive_runtime;
use semantics::Tag;

type Handlers = Arc<[HandlerEntry]>;

/// Converts a shared element tree into a Leptos view, including its stylesheet.
///
/// This supports server rendering, hydration, and reactive client rendering.
/// Classes override typed base styles regardless of builder method order.
/// Existing event handlers receive platform-neutral events and stop DOM
/// bubbling when they return [`EventOutcome::Handled`].
/// Controlled inputs propose changes; the application owns their committed
/// state. A rejected proposal is restored after the event. Radio group names
/// are scoped per mounted tree. Server markup omits those names until
/// hydration, so radios become a keyboard group when the client attaches their
/// handlers.
///
/// # Panics
///
/// Panics for platform-specific `Custom` widgets or an invalid control tree
/// (see [`Element::validate`]).
pub fn render(element: Element) -> AnyView {
	element
		.validate()
		.unwrap_or_else(|error| panic!("invalid leptoskit element tree: {error}"));
	let wants_focus = asks_for_focus(&element);
	let scrolls = sets_scroll_offset(&element);
	let mut sheets = BTreeMap::new();
	let root = render_node(element, true, true, false, &[], &mut sheets);
	let css = sheets.into_values().collect::<String>();

	if wants_focus {
		focus_after_render();
	}

	if scrolls {
		scroll_after_render();
	}

	(html::style().child(css), root).into_any()
}

/// Whether any element in the tree asks for keyboard focus.
fn asks_for_focus(element: &Element) -> bool {
	element.props.focused == Some(true) || element.children.iter().any(asks_for_focus)
}

/// Whether any element in the tree sets a scroll offset.
fn sets_scroll_offset(element: &Element) -> bool {
	element.props.scroll_offset.is_some() || element.children.iter().any(sets_scroll_offset)
}

/// The `data-lk-scroll` value naming a scroll target, which the renderer
/// compares with the last target it applied to the same DOM element.
fn scroll_target(offset: crate::ScrollOffset) -> String {
	let animated = if offset.animated { " smooth" } else { "" };
	format!("{} {}{animated}", offset.x, offset.y)
}

/// Scrolls every element whose target changed since the renderer last
/// scrolled it, once the new DOM is in place: in a microtask, so a target set
/// from an animation frame scrolls before that frame paints. The applied
/// target is kept on the element, so a rebuild that keeps the DOM node leaves
/// the person's own scrolling alone, and a new node scrolls to its target.
fn scroll_after_render() {
	#[cfg(target_arch = "wasm32")]
	queue_microtask(|| {
		const APPLIED: &str = "data-lk-scrolled";

		let Some(document) = web_sys::window().and_then(|window| window.document()) else {
			return;
		};
		let Ok(targets) = document.query_selector_all("[data-lk-scroll]") else {
			return;
		};

		for index in 0..targets.length() {
			let Some(element) = targets
				.item(index)
				.and_then(|node| node.dyn_into::<web_sys::Element>().ok())
			else {
				continue;
			};
			let Some(target) = element.get_attribute("data-lk-scroll") else {
				continue;
			};

			if element.get_attribute(APPLIED).as_deref() == Some(target.as_str()) {
				continue;
			}

			let mut parts = target.split(' ');
			let (Some(Ok(x)), Some(Ok(y))) = (
				parts.next().map(str::parse::<f64>),
				parts.next().map(str::parse::<f64>),
			) else {
				continue;
			};
			let options = web_sys::ScrollToOptions::new();
			options.set_left(x);
			options.set_top(y);
			options.set_behavior(if parts.next() == Some("smooth") {
				web_sys::ScrollBehavior::Smooth
			} else {
				web_sys::ScrollBehavior::Instant
			});
			element.scroll_to_with_scroll_to_options(&options);
			let _ = element.set_attribute(APPLIED, &target);
		}
	});
}

/// Moves focus to the last element asking for it, once the browser has the
/// new DOM, unless it already holds focus.
fn focus_after_render() {
	// Scheduling fails only without a window, and then nothing can take focus.
	#[cfg(target_arch = "wasm32")]
	let _ = request_animation_frame(|| {
		let Some(document) = web_sys::window().and_then(|window| window.document()) else {
			return;
		};
		let Ok(candidates) = document.query_selector_all("[data-lk-focus]") else {
			return;
		};
		let target = candidates
			.length()
			.checked_sub(1)
			.and_then(|last| candidates.item(last))
			.and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok());

		if let Some(target) = target {
			let active = document.active_element();
			if active.as_ref() != Some(target.as_ref()) {
				let _ = target.focus();
			}
		}
	});
}

/// What a node's typed DOM element is built from, besides the attributes
/// every kind shares.
struct NodeParts {
	kind: NativeKind,
	tag: Tag,
	props: NativeProps,
	text: String,
	/// What a text-bearing element shows: its text, or its styled runs.
	content: AnyView,
	children: Vec<AnyView>,
	enabled: bool,
	/// The target of a pressable that renders as a link.
	href: Option<String>,
	label: Option<String>,
	/// The handlers of this node and its ancestors, innermost last.
	chain: Vec<Handlers>,
}

/// Builds each typed DOM control before adding attributes common to all kinds.
fn render_node(
	element: Element,
	root: bool,
	parent_enabled: bool,
	parent_hidden: bool,
	ancestors: &[Handlers],
	sheets: &mut BTreeMap<String, String>,
) -> AnyView {
	let (scope, sheet) = scoped_sheet(&element, root);
	let handlers: Handlers = element.handlers().to_vec().into();
	let mut chain = ancestors.to_vec();
	chain.push(Arc::clone(&handlers));
	let classes = element.classes().classes().to_owned();
	let kind = element.kind;
	let mut props = element.props;
	let id = props.id.take();
	let test_id = props.test_id().map(str::to_owned);
	let enabled = parent_enabled && props.enabled.unwrap_or(true);
	// A pressable with an href is a link: an `<a>` the browser can open in
	// a new tab, whose plain clicks the app handles in place.
	let href = (kind == NativeKind::Pressable)
		.then(|| props.href().map(str::to_owned))
		.flatten();
	let link = href.is_some();
	// Hidden subtrees stay hidden: assistive technology ignores descendants of
	// an `aria-hidden` element, and the attribute on each node keeps the
	// element tree and the DOM in agreement.
	let hidden = parent_hidden || props.aria().hidden == Some(true);
	let named = props.names_itself();
	// An unnamed drawing is decoration, as an image with empty alt text is.
	let decorative = kind == NativeKind::Vector && !named && props.aria().role.is_none();
	let tag = semantics::tag(kind, props.aria(), named, element.children.is_empty());
	let label = props
		.accessibility_label
		.clone()
		.or_else(|| (kind == NativeKind::ActivityIndicator).then(|| "Loading".to_owned()));
	let text = props.text.clone().unwrap_or_default();
	// Styled runs render as inline spans; plain text is one text node.
	let content = match props.spans.take() {
		Some(spans) => spans_view(spans),
		None => text.clone().into_any(),
	};
	let custom_control = semantics::custom_control(kind, props.aria());

	sheets.entry(scope.clone()).or_insert(sheet);
	let children = element
		.children
		.into_iter()
		.map(|child| render_node(child, false, enabled, hidden, &chain, sheets))
		.collect::<Vec<_>>();
	let focused = props.focused == Some(true);
	let scroll = props.scroll_offset.map(scroll_target);

	let attrs = (
		custom_attribute("data-lk-style", scope),
		custom_attribute("data-lk-focus", focused.then_some("true")),
		custom_attribute("data-lk-scroll", scroll),
		custom_attribute("class", classes),
		custom_attribute("aria-label", label.clone()),
		custom_attribute("aria-disabled", (!enabled).then_some("true")),
		custom_attribute("data-testid", test_id),
		semantics::attributes(
			id,
			props.aria(),
			kind,
			tag,
			hidden || decorative,
			custom_control,
		),
		listeners(handlers, chain.clone(), link, custom_control),
	);
	let node = typed_element(
		NodeParts {
			kind,
			tag,
			props,
			text,
			content,
			children,
			enabled,
			href,
			label,
			chain,
		},
		attrs,
	);

	if root {
		node.directive(
			|element: web_sys::Element| {
				controls::scope_root(&element);
				controls::enable_touch_active(&element);
			},
			(),
		)
		.into_any()
	} else {
		node
	}
}

/// The node's style scope and its stylesheet: typed base styles, utility
/// classes, and control decoration, under a selector derived from their
/// content so equal styles share one sheet.
fn scoped_sheet(element: &Element, root: bool) -> (String, String) {
	let base = styles::base(element, root);
	let styles_active = element.classes().active_class().is_some();
	let fingerprint = format!(
		"{base}\0{}{}",
		element.classes().css(),
		styles::control("", element.kind, styles_active)
	);
	let scope = format!("lk-{:x}", Sha256::digest(fingerprint.as_bytes()));
	let selector = format!("[data-lk-style=\"{scope}\"]");
	let sheet = format!(
		":where({selector}){{{base}}}{}{}",
		element.classes().css_scoped(&selector),
		styles::control(&selector, element.kind, styles_active),
	);
	(scope, sheet)
}

/// The DOM listeners every node carries, which translate browser events into
/// platform-neutral ones for the node's handlers.
fn listeners(
	handlers: Handlers,
	scroll_chain: Vec<Handlers>,
	link: bool,
	custom_control: bool,
) -> impl Attribute {
	let listens_for_keys = handlers
		.iter()
		.any(|entry| entry.event == crate::NativeEventSlot::Key);
	let press = Arc::clone(&handlers);
	let keys = Arc::clone(&handlers);
	let key_press = Arc::clone(&handlers);
	let submit = Arc::clone(&handlers);
	let focus = Arc::clone(&handlers);
	let blur = handlers;

	(
		on(ev::click, move |event| {
			if !controls::interactive(&event) || controls::typed_target(&event) {
				return;
			}
			if link && controls::modified(&event) {
				return;
			}

			let outcome = dispatch(&press, &NativeEvent::Press, &event);
			// A handled link click stays in the app; an unhandled one is
			// followed by the browser.
			if link && outcome == EventOutcome::Handled {
				event.prevent_default();
			}
		}),
		on(ev::keydown, move |event| {
			if !controls::interactive(event.as_ref()) {
				return;
			}

			// Key handlers hear every key inside them, typing included, before
			// the focused control acts on it. A handled key loses its browser
			// meaning, so ctrl+p does not print. Command-key shortcuts
			// stay the browser's.
			if listens_for_keys && !event.is_composing() && !event.meta_key() {
				let native = NativeEvent::Key(crate::KeyPress {
					key: event.key(),
					ctrl: event.ctrl_key(),
					alt: event.alt_key(),
					shift: event.shift_key(),
				});

				if dispatch(&keys, &native, event.as_ref()) == EventOutcome::Handled {
					event.prevent_default();
					return;
				}
			}

			if controls::typed_target(event.as_ref()) {
				return;
			}

			let input = event
				.target()
				.and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok());

			if let Some(input) = input.filter(|_| event.key() == "Enter" && !event.is_composing()) {
				let native = NativeEvent::Submit {
					value: input.value(),
				};

				if dispatch(&submit, &native, event.as_ref()) == EventOutcome::Handled {
					event.prevent_default();
				}

				return;
			}

			// A container with a control role has no native activation, so
			// Enter and Space press it, as they do a `<button>`. Only the
			// focused element itself activates, not an ancestor control.
			if custom_control
				&& matches!(event.key().as_str(), "Enter" | " ")
				&& !event.is_composing()
				&& event.target() == event.current_target()
				&& dispatch(&key_press, &NativeEvent::Press, event.as_ref())
					== EventOutcome::Handled
			{
				event.prevent_default();
			}
		}),
		on(ev::focusin, move |event| {
			if !controls::interactive(event.as_ref()) {
				return;
			}

			dispatch(&focus, &NativeEvent::Focus, event.as_ref());
		}),
		on(ev::focusout, move |event| {
			if !controls::interactive(event.as_ref()) {
				return;
			}

			dispatch(&blur, &NativeEvent::Blur, event.as_ref());
		}),
		on(ev::scroll, move |event| {
			if !controls::interactive(&event) {
				return;
			}

			let target = event
				.target()
				.and_then(|target| target.dyn_into::<web_sys::Element>().ok())
				.expect("scroll events from leptoskit controls target DOM elements");
			let native = NativeEvent::Scroll {
				offset_x: target.scroll_left() as f32,
				offset_y: target.scroll_top() as f32,
			};

			// Browser scroll does not bubble, so walk the native ancestor
			// chain.
			for handlers in scroll_chain.iter().rev() {
				if dispatch(handlers, &native, &event) == EventOutcome::Handled {
					break;
				}
			}

			event.stop_propagation();
		}),
	)
}

/// The DOM element for a node's kind, with the shared attributes added.
///
/// Changing handlers attach to typed elements before erasure. In the pinned
/// Tachys version, rebuilding attributes spread onto `AnyView` keeps the
/// original listener state instead of replacing its callbacks.
fn typed_element(parts: NodeParts, attrs: impl Attribute + 'static) -> AnyView {
	let NodeParts {
		kind,
		tag,
		props,
		text,
		content,
		children,
		enabled,
		href,
		label,
		chain,
	} = parts;

	match kind {
		NativeKind::View | NativeKind::ScrollView => {
			match tag {
				Tag::Div => {
					html::div()
						.child((content, children))
						.add_any_attr(attrs)
						.into_any()
				}
				Tag::Rule => html::hr().add_any_attr(attrs).into_any(),
				semantic => {
					custom(semantic.name())
						.child((content, children))
						.add_any_attr(attrs)
						.into_any()
				}
			}
		}
		NativeKind::Text | NativeKind::TextRun => {
			match tag {
				Tag::Heading(_) => {
					custom(tag.name())
						.child((content, children))
						.add_any_attr(attrs)
						.into_any()
				}
				_ => {
					html::span()
						.child((content, children))
						.add_any_attr(attrs)
						.into_any()
				}
			}
		}
		NativeKind::Button => {
			html::button()
				.r#type("button")
				.disabled(!enabled)
				.child((content, children))
				.add_any_attr(attrs)
				.into_any()
		}
		NativeKind::Pressable => {
			match href {
				// A disabled link has no target, so it cannot be followed.
				Some(href) => {
					html::a()
						.href(enabled.then_some(href))
						.child(children)
						.add_any_attr(attrs)
						.into_any()
				}
				None => {
					html::button()
						.r#type("button")
						.disabled(!enabled)
						.child(children)
						.add_any_attr(attrs)
						.into_any()
				}
			}
		}
		NativeKind::Image => {
			html::img()
				.src(props.source.unwrap_or_default())
				.alt(label.unwrap_or_default())
				.add_any_attr(attrs)
				.into_any()
		}
		NativeKind::Rive => {
			#[cfg(feature = "rive")]
			{
				html::canvas()
					.add_any_attr(attrs)
					.add_any_attr(rive::Binding {
						content: *props.rive.expect("validated Rive content"),
						running: props.running == Some(true),
					})
					.into_any()
			}
			#[cfg(not(feature = "rive"))]
			panic!("enable leptoskit_native's rive feature before rendering a Rive element")
		}
		NativeKind::TextInput | NativeKind::PasswordInput => {
			text_input(kind, props, text, enabled, chain, attrs)
		}
		NativeKind::Switch | NativeKind::Checkbox | NativeKind::Radio => {
			controls::toggle(
				props.checked.unwrap_or(false),
				enabled,
				if kind == NativeKind::Radio {
					props.radio_group.map(|group| group.name().to_owned())
				} else {
					None
				},
				kind == NativeKind::Switch,
				chain,
			)
			.add_any_attr(attrs)
			.into_any()
		}
		NativeKind::Slider => {
			controls::slider(props.slider.unwrap_or_default(), enabled, chain)
				.add_any_attr(attrs)
				.into_any()
		}
		NativeKind::LinearProgressIndicator => {
			let value = match props.progress.unwrap_or_default() {
				Progress::Indeterminate => None,
				Progress::Determinate(value) => Some(value.get().to_string()),
			};
			html::progress()
				.max("1")
				.value(value)
				.add_any_attr(attrs)
				.into_any()
		}
		NativeKind::ActivityIndicator => activity_indicator(props.running.unwrap_or(true), attrs),
		NativeKind::Vector => {
			html::span()
				.attr(
					"role",
					(props.names_itself() && props.aria().role.is_none()).then_some("img"),
				)
				.child(vector::svg(props.drawing.as_ref()))
				.add_any_attr(attrs)
				.into_any()
		}
		NativeKind::Custom(name) => {
			panic!(
				"leptoskit web renderer does not support platform-specific Custom widget `{name}`"
			)
		}
	}
}

/// A text or password `<input>` whose edits propose values to the
/// application, which owns the committed value.
fn text_input(
	kind: NativeKind,
	props: NativeProps,
	text: String,
	enabled: bool,
	chain: Vec<Handlers>,
	attrs: impl Attribute + 'static,
) -> AnyView {
	let value = props.value.unwrap_or(text);
	html::input()
		.r#type(if kind == NativeKind::PasswordInput {
			"password"
		} else {
			"text"
		})
		.disabled(!enabled)
		.placeholder(props.placeholder.unwrap_or_default())
		.attr("autocomplete", props.autocomplete)
		.value(value.clone())
		.attr("data-lk-value", value)
		.directive(controls::restore_input, ())
		.on(ev::input, move |event| {
			controls::text_event(&event, &chain);
		})
		.add_any_attr(attrs)
		.into_any()
}

/// A spinner whose animation and accessibility follow whether it is running.
fn activity_indicator(running: bool, attrs: impl Attribute + 'static) -> AnyView {
	let flag = if running { "true" } else { "false" };
	html::span()
		.role("progressbar")
		.attr("aria-hidden", (!running).then_some("true"))
		.attr("aria-busy", flag)
		.attr("data-lk-running", flag)
		.add_any_attr(attrs)
		.into_any()
}

/// Mirrors `NativeSurface`: invoke every matching handler on the current node.
fn dispatch(
	handlers: &[HandlerEntry],
	native: &NativeEvent,
	event: &web_sys::Event,
) -> EventOutcome {
	let mut outcome = EventOutcome::Unhandled;

	for entry in handlers {
		if entry.event.matches(native) && (entry.handler)(native) == EventOutcome::Handled {
			outcome = EventOutcome::Handled;
		}
	}

	if outcome == EventOutcome::Handled {
		event.stop_propagation();
	}

	outcome
}

/// A `Text`'s styled runs as inline spans, in reading order.
fn spans_view(spans: Vec<crate::TextSpan>) -> AnyView {
	spans
		.into_iter()
		.map(|span| {
			let style = styles::span(&span);
			html::span().child(span.text).add_any_attr(custom_attribute(
				"style",
				(!style.is_empty()).then_some(style),
			))
		})
		.collect_view()
		.into_any()
}
