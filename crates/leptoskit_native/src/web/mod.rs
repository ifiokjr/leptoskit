//! Render shared leptoskit elements as genuine Leptos DOM views.
//!
//! Pass [`render`] the same [`Element`] used with a native surface. Wrap the
//! call in a reactive closure to rebuild when Leptos signals change. Leptos
//! reconciles existing DOM nodes, preserving focus and input selection.
//!
//! Styles have deterministic, content-derived scopes, so server rendering and
//! hydration agree and independently rendered trees cannot affect one another's
//! class precedence. Browser media queries update responsive classes directly.

use std::collections::BTreeMap;
use std::sync::Arc;

use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use leptos::tachys::html::attribute::custom::custom_attribute;
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
mod styles;

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
	let root = render_node(element, true, true, &[], &mut sheets);
	let css = sheets.into_values().collect::<String>();

	(html::style().child(css), root).into_any()
}

/// Builds each typed DOM control before adding attributes common to all kinds.
fn render_node(
	element: Element,
	root: bool,
	parent_enabled: bool,
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
	let props = element.props;
	let enabled = parent_enabled && props.enabled.unwrap_or(true) && props.hidden != Some(true);
	let label = props.accessibility_label;
	let text = props.text.unwrap_or_default();
	let kind = element.kind;

	sheets.entry(scope.clone()).or_insert(sheet);
	let children = element
		.children
		.into_iter()
		.map(|child| render_node(child, false, enabled, &chain, sheets))
		.collect::<Vec<_>>();
	let node = match kind {
		NativeKind::View | NativeKind::ScrollView => html::div().child((text, children)).into_any(),
		NativeKind::Text | NativeKind::TextRun => html::span().child((text, children)).into_any(),
		NativeKind::Button => {
			html::button()
				.r#type("button")
				.disabled(!enabled)
				.child((text, children))
				.into_any()
		}
		NativeKind::Image => {
			html::img()
				.src(props.source.unwrap_or_default())
				.alt(label.clone().unwrap_or_default())
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
					controls::text_event(&event, enabled, &text_chain);
				})
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
				chain.clone(),
			)
		}
		NativeKind::Slider => {
			controls::slider(props.slider.unwrap_or_default(), enabled, chain.clone())
		}
		NativeKind::LinearProgressIndicator => {
			let value = match props.progress.unwrap_or_default() {
				Progress::Indeterminate => None,
				Progress::Determinate(value) => Some(value.get().to_string()),
			};

			html::progress().max("1").value(value).into_any()
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
				.into_any()
		}
		NativeKind::Custom(name) => {
			panic!(
				"leptoskit web renderer does not support platform-specific Custom widget `{name}`"
			)
		}
	};
	let press = Arc::clone(&handlers);
	let submit = Arc::clone(&handlers);
	let focus = Arc::clone(&handlers);
	let blur = handlers;

	let node = node
		.add_any_attr((
			custom_attribute("data-lk-style", scope),
			custom_attribute("class", classes),
			custom_attribute("aria-label", label),
			custom_attribute("aria-disabled", (!enabled).then_some("true")),
			custom_attribute(
				"data-lk-hidden",
				props
					.hidden
					.map(|hidden| if hidden { "true" } else { "false" }),
			),
			on(ev::click, move |event| {
				if !controls::interactive(&event, enabled) || controls::typed_target(&event) {
					return;
				}

				dispatch(&press, &NativeEvent::Press, &event);
			}),
			on(ev::keydown, move |event| {
				if !controls::interactive(event.as_ref(), enabled)
					|| controls::typed_target(event.as_ref())
				{
					return;
				}

				let input = event
					.target()
					.and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok());

				if let Some(input) =
					input.filter(|_| event.key() == "Enter" && !event.is_composing())
				{
					let native = NativeEvent::Submit {
						value: input.value(),
					};

					if dispatch(&submit, &native, event.as_ref()) == EventOutcome::Handled {
						event.prevent_default();
					}
				}
			}),
			on(ev::focusin, move |event| {
				if !controls::interactive(event.as_ref(), enabled) {
					return;
				}

				dispatch(&focus, &NativeEvent::Focus, event.as_ref());
			}),
			on(ev::focusout, move |event| {
				if !controls::interactive(event.as_ref(), enabled) {
					return;
				}

				dispatch(&blur, &NativeEvent::Blur, event.as_ref());
			}),
			on(ev::scroll, move |event| {
				if !controls::interactive(&event, enabled) {
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
				for handlers in chain.iter().rev() {
					if dispatch(handlers, &native, &event) == EventOutcome::Handled {
						break;
					}
				}

				event.stop_propagation();
			}),
		))
		.into_any();

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
