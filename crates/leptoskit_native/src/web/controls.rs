//! DOM state and proposal events for controlled widgets.

use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::Handlers;
use super::dispatch;
use crate::NativeEvent;
use crate::SliderState;

static NEXT_ROOT: AtomicUsize = AtomicUsize::new(1);

/// Names exist only after mounting: server trees cannot interfere with each
/// other's initial radio selections, and hydration needs no request-global ID.
pub(super) fn scope_root(element: &web_sys::Element) {
	let id = element
		.get_attribute("data-lk-radio-scope")
		.and_then(|value| value.parse().ok())
		.unwrap_or_else(|| NEXT_ROOT.fetch_add(1, Ordering::Relaxed));
	element
		.set_attribute("data-lk-radio-scope", &id.to_string())
		.expect("set renderer radio scope");
	let nodes = element
		.query_selector_all("input[type=radio]")
		.expect("radio selector");
	let root_radio = element
		.dyn_ref::<web_sys::HtmlInputElement>()
		.filter(|input| input.type_() == "radio")
		.cloned();
	let radios = root_radio
		.into_iter()
		.chain((0..nodes.length()).map(|index| {
			nodes
				.item(index)
				.expect("radio exists")
				.unchecked_into::<web_sys::HtmlInputElement>()
		}))
		.filter(|radio| {
			radio
				.closest("[data-lk-radio-scope]")
				.expect("scope selector")
				.as_ref()
				== Some(element)
		})
		.collect::<Vec<_>>();

	// AnyView attaches spread attributes after building its descendants, so
	// every radio is present before we assign names in the retained root scope.
	for radio in &radios {
		let group = radio
			.get_attribute("data-lk-radio-group")
			.expect("validated radio group");
		radio.set_name(&format!("lk-radio-{id}-{group}"));
	}

	for checked in [false, true] {
		for radio in &radios {
			if radio.get_attribute("data-lk-checked").as_deref()
				== Some(if checked { "true" } else { "false" })
			{
				radio.set_checked(checked);
			}
		}
	}
}

thread_local! {
	/// One listener for every root. The root directive runs again on each
	/// rebuild, and the DOM ignores a listener an element already holds.
	static TOUCH_LISTENER: wasm_bindgen::JsValue =
		wasm_bindgen::closure::Closure::<dyn Fn()>::new(|| {}).into_js_value();
}

/// Lets `active:` styles show on touch screens.
///
/// iOS Safari marks a touched element `:active` only inside a region that
/// listens for touches; without one, it synthesizes the click but never
/// shows the press. A passive listener that does nothing opts the
/// rendered tree in without delaying scrolling.
pub(super) fn enable_touch_active(element: &web_sys::Element) {
	let options = web_sys::AddEventListenerOptions::new();
	options.set_passive(true);
	TOUCH_LISTENER.with(|listener| {
		element
			.add_event_listener_with_callback_and_add_event_listener_options(
				"touchstart",
				listener.unchecked_ref(),
				&options,
			)
			.expect("a rendered root accepts a passive touch listener");
	});
}

/// Always write controlled properties, even if their committed value has not
/// changed. Leptos's ordinary property diff cannot undo a rejected proposal.
pub(super) fn restore_input(element: web_sys::Element) {
	let input: web_sys::HtmlInputElement = element.unchecked_into();

	restore(&input);
}

pub(super) fn restore(input: &web_sys::HtmlInputElement) {
	if let Some(value) = input.get_attribute("data-lk-checked") {
		input.set_checked(value == "true");
	}

	if let Some(value) = input.get_attribute("data-lk-value")
		&& input.value() != value
	{
		input.set_value(&value);
	}
}

/// Restores both the proposed option and the option the browser unchecked.
fn restore_radio_group(input: &web_sys::HtmlInputElement) {
	restore(input);
	let Some(root) = input
		.closest("[data-lk-radio-scope]")
		.expect("valid scope selector")
	else {
		return;
	};
	let radios = root
		.query_selector_all("input[type=radio]")
		.expect("valid radio selector");

	for checked in [false, true] {
		for index in 0..radios.length() {
			let radio: web_sys::HtmlInputElement =
				radios.item(index).expect("radio exists").unchecked_into();

			if radio.name() == input.name()
				&& radio.get_attribute("data-lk-checked").as_deref()
					== Some(if checked { "true" } else { "false" })
			{
				radio.set_checked(checked);
			}
		}
	}
}

/// Hidden or disabled ancestors reject synthetic events as well as user input.
pub(super) fn interactive(event: &web_sys::Event) -> bool {
	let target = event
		.target()
		.and_then(|target| target.dyn_into::<web_sys::Element>().ok());
	let visible = target.as_ref().is_some_and(visible);

	if !visible {
		event.stop_propagation();
	}

	visible
}

fn visible(element: &web_sys::Element) -> bool {
	let window = web_sys::window().expect("browser window");
	let mut target = Some(element.clone());

	while let Some(element) = target {
		let disabled = element.has_attribute("disabled")
			|| element.get_attribute("aria-disabled").as_deref() == Some("true");
		let hidden_style = window
			.get_computed_style(&element)
			.expect("computed style")
			.is_some_and(|style| {
				style
					.get_property_value("display")
					.expect("display property")
					== "none"
					|| matches!(
						style
							.get_property_value("visibility")
							.expect("visibility property")
							.as_str(),
						"hidden" | "collapse"
					)
			});

		if hidden_style || disabled {
			return false;
		}

		target = element.parent_element();
	}

	true
}

/// Typed input events walk the native chain once, then stop DOM bubbling.
pub(super) fn input_event(event: &web_sys::Event, chain: &[Handlers], slider: Option<SliderState>) {
	let input = event
		.target()
		.and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
		.expect("control input event targets an input");

	if interactive(event) {
		let native = if let Some(slider) = slider {
			slider
				.proposed_value(input.value_as_number())
				.map(|value| NativeEvent::ValueChange { value })
		} else {
			Some(NativeEvent::Toggle {
				checked: input.checked(),
			})
		};

		if let Some(native) = native {
			for handlers in chain.iter().rev() {
				if dispatch(handlers, &native, event) == crate::EventOutcome::Handled {
					break;
				}
			}
		}
	}

	if input.type_() == "radio" {
		restore_radio_group(&input);
	} else {
		restore(&input);
	}

	event.stop_propagation();
}

/// Whether a click asks the browser to open a link its own way: in a new tab
/// or window, or as a download, rather than in place.
pub(super) fn modified(event: &web_sys::MouseEvent) -> bool {
	event.button() != 0
		|| event.ctrl_key()
		|| event.meta_key()
		|| event.shift_key()
		|| event.alt_key()
}

/// Typed controls do not also emit string changes or generic presses.
pub(super) fn typed_target(event: &web_sys::Event) -> bool {
	event
		.target()
		.and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
		.is_some_and(|input| matches!(input.type_().as_str(), "checkbox" | "radio" | "range"))
}

pub(super) fn text_event(event: &web_sys::Event, chain: &[Handlers]) {
	let input = event
		.target()
		.and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
		.expect("text input event targets an input");

	if interactive(event) {
		let native = NativeEvent::Change {
			value: input.value(),
		};

		for handlers in chain.iter().rev() {
			if dispatch(handlers, &native, event) == crate::EventOutcome::Handled {
				break;
			}
		}
	}

	queue_microtask(move || restore(&input));
	event.stop_propagation();
}

pub(super) fn toggle(
	checked: bool,
	enabled: bool,
	radio: Option<String>,
	is_switch: bool,
	chain: Vec<Handlers>,
) -> impl RenderHtml {
	html::input()
		.r#type(if radio.is_some() { "radio" } else { "checkbox" })
		.role(is_switch.then_some("switch"))
		.disabled(!enabled)
		.checked(checked)
		.attr("data-lk-checked", if checked { "true" } else { "false" })
		.attr("data-lk-radio-group", radio)
		.directive(restore_input, ())
		.on(leptos::ev::input, move |event| {
			input_event(&event, &chain, None);
		})
		.on(leptos::ev::keydown, move |event| radio_key(&event))
}

/// Native and browser radios use the same enabled-option traversal policy.
fn radio_key(event: &web_sys::KeyboardEvent) {
	let input = event
		.target()
		.and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
		.expect("radio keyboard event targets an input");
	let backwards = match event.key().as_str() {
		"ArrowLeft" | "ArrowUp" => true,
		"ArrowRight" | "ArrowDown" => false,
		_ => return,
	};

	if input.type_() != "radio" || !interactive(event.as_ref()) {
		return;
	}

	event.prevent_default();
	let root = input
		.closest("[data-lk-radio-scope]")
		.expect("scope selector")
		.expect("mounted radio root");
	let nodes = root
		.query_selector_all("input[type=radio]")
		.expect("radio selector");
	let options = (0..nodes.length())
		.map(|index| {
			nodes
				.item(index)
				.expect("radio exists")
				.unchecked_into::<web_sys::HtmlInputElement>()
		})
		.filter(|radio| radio.name() == input.name() && !radio.disabled())
		.filter(|radio| visible(radio))
		.collect::<Vec<_>>();
	let Some(index) = options.iter().position(|radio| radio == &input) else {
		return;
	};
	let index = if backwards {
		(index + options.len() - 1) % options.len()
	} else {
		(index + 1) % options.len()
	};
	let next = &options[index];

	next.focus().expect("focus next radio");
	next.click();
}

pub(super) fn slider(state: SliderState, enabled: bool, chain: Vec<Handlers>) -> impl RenderHtml {
	html::input()
		.r#type("range")
		.disabled(!enabled)
		.min(state.min().to_string())
		.max(state.max().to_string())
		.step(
			state
				.step()
				.map_or_else(|| "any".to_owned(), |step| step.to_string()),
		)
		.value(state.value().to_string())
		.attr("data-lk-value", state.value().to_string())
		.directive(restore_input, ())
		.on(leptos::ev::input, move |event| {
			input_event(&event, &chain, Some(state));
		})
}
