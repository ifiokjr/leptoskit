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
use crate::Progress;
use crate::element::HandlerEntry;

mod controls;
mod semantics;
mod styles;

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
	let mut sheets = BTreeMap::new();
	let root = render_node(element, true, true, false, &[], &mut sheets);
	let css = sheets.into_values().collect::<String>();

	(html::style().child(css), root).into_any()
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
	let base = styles::base(&element, root);
	let fingerprint = format!(
		"{base}\0{}{}",
		element.classes().css(),
		styles::control("", element.kind)
	);
	let scope = format!("lk-{:x}", Sha256::digest(fingerprint.as_bytes()));
	let selector = format!("[data-lk-style=\"{scope}\"]");
	let sheet = format!(
		":where({selector}){{{base}}}{}{}",
		element.classes().css_scoped(&selector),
		styles::control(&selector, element.kind),
	);
	let handlers: Handlers = element.handlers().to_vec().into();
	let mut chain = ancestors.to_vec();
	chain.push(Arc::clone(&handlers));
	let classes = element.classes().classes().to_owned();
	let kind = element.kind;
	let mut props = element.props;
	let aria = std::mem::take(&mut props.aria);
	let id = props.id.take();
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
	let hidden = parent_hidden || aria.hidden == Some(true);
	let label = aria
		.label
		.clone()
		.or_else(|| (kind == NativeKind::ActivityIndicator).then(|| "Loading".to_owned()));
	let text = props.text.unwrap_or_default();
	let tag = semantics::tag(kind, &aria, element.children.is_empty());
	let custom_control = semantics::custom_control(kind, &aria);

	sheets.entry(scope.clone()).or_insert(sheet);
	let children = element
		.children
		.into_iter()
		.map(|child| render_node(child, false, enabled, hidden, &chain, sheets))
		.collect::<Vec<_>>();
	let press = Arc::clone(&handlers);
	let key_press = Arc::clone(&handlers);
	let submit = Arc::clone(&handlers);
	let focus = Arc::clone(&handlers);
	let blur = handlers;
	let scroll_chain = chain.clone();

	let attrs = (
		custom_attribute("data-lk-style", scope),
		custom_attribute("class", classes),
		custom_attribute("aria-label", label.clone()),
		custom_attribute("aria-disabled", (!enabled).then_some("true")),
		semantics::attributes(id, &aria, kind, tag, hidden, custom_control),
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
			if !controls::interactive(event.as_ref()) || controls::typed_target(event.as_ref()) {
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
	);

	// Attach changing handlers to typed elements before erasure. In the
	// pinned Tachys version, rebuilding attributes spread onto AnyView keeps
	// the original listener state instead of replacing its callbacks.
	let node = match kind {
		NativeKind::View | NativeKind::ScrollView => {
			match tag {
				Tag::Div => {
					html::div()
						.child((text, children))
						.add_any_attr(attrs)
						.into_any()
				}
				Tag::Rule => html::hr().add_any_attr(attrs).into_any(),
				semantic => {
					custom(semantic.name())
						.child((text, children))
						.add_any_attr(attrs)
						.into_any()
				}
			}
		}
		NativeKind::Text | NativeKind::TextRun => {
			match tag {
				Tag::Heading(_) => {
					custom(tag.name())
						.child((text, children))
						.add_any_attr(attrs)
						.into_any()
				}
				_ => {
					html::span()
						.child((text, children))
						.add_any_attr(attrs)
						.into_any()
				}
			}
		}
		NativeKind::Button => {
			html::button()
				.r#type("button")
				.disabled(!enabled)
				.child((text, children))
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
		NativeKind::TextInput | NativeKind::PasswordInput => {
			let value = props.value.unwrap_or(text);
			let text_chain = chain.clone();
			html::input()
				.r#type(if kind == NativeKind::PasswordInput {
					"password"
				} else {
					"text"
				})
				.disabled(!enabled)
				.placeholder(props.placeholder.unwrap_or_default())
				.value(value.clone())
				.attr("data-lk-value", value)
				.directive(controls::restore_input, ())
				.on(ev::input, move |event| {
					controls::text_event(&event, &text_chain);
				})
				.add_any_attr(attrs)
				.into_any()
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
		NativeKind::ActivityIndicator => {
			html::span()
				.role("progressbar")
				.attr(
					"aria-hidden",
					(!props.running.unwrap_or(true)).then_some("true"),
				)
				.attr(
					"aria-busy",
					if props.running.unwrap_or(true) {
						"true"
					} else {
						"false"
					},
				)
				.attr(
					"data-lk-running",
					if props.running.unwrap_or(true) {
						"true"
					} else {
						"false"
					},
				)
				.add_any_attr(attrs)
				.into_any()
		}
		NativeKind::Custom(name) => {
			panic!(
				"leptoskit web renderer does not support platform-specific Custom widget `{name}`"
			)
		}
	};

	if root {
		node.directive(
			|element: web_sys::Element| controls::scope_root(&element),
			(),
		)
		.into_any()
	} else {
		node
	}
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
