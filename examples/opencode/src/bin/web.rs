//! `opencode-web`: the same interface in the browser, through Leptos.
//!
//! The element tree is the one the terminal draws. The page sets a
//! monospace font whose cells are 8×16 pixels, so the browser's CSS layout
//! lands on the same grid the terminal backend lays out with Yoga.

#[cfg(target_arch = "wasm32")]
fn main() {
	use std::sync::Arc;

	use leptos::ev;
	use leptos::prelude::*;
	use opencode_example::Dispatch;
	use opencode_example::Opencode;
	use opencode_example::ShortcutKey;
	use opencode_example::shortcut;

	/// The window's size in 8×16 cells.
	fn cells() -> (u16, u16) {
		let window = window();
		let size = |value: Result<wasm_bindgen::JsValue, _>, cell: f64| {
			(value.ok().and_then(|value| value.as_f64()).unwrap_or(0.0) / cell) as u16
		};
		(
			size(window.inner_width(), 8.0),
			size(window.inner_height(), 16.0),
		)
	}

	fn shortcut_key(event: &web_sys::KeyboardEvent) -> Option<ShortcutKey> {
		Some(match event.key().as_str() {
			"Escape" => ShortcutKey::Escape,
			"ArrowUp" => ShortcutKey::Up,
			"ArrowDown" => ShortcutKey::Down,
			"Enter" => ShortcutKey::Enter,
			"Tab" if event.shift_key() => ShortcutKey::BackTab,
			key => {
				let mut symbols = key.chars();
				match (symbols.next(), symbols.next()) {
					(Some(symbol), None) => ShortcutKey::Char(symbol.to_ascii_lowercase()),
					_ => return None,
				}
			}
		})
	}

	let state = RwSignal::new(Opencode::new("~/code/clockwork:main"));
	state.update(|state| state.size = cells());
	let dispatch: Dispatch = Arc::new(move |action| state.update(|state| state.apply(action)));

	mount_to_body(move || {
		let _resize = window_event_listener(ev::resize, move |_| {
			state.update(|state| state.size = cells());
		});
		// ctrl+p would print and shift+tab would leave the prompt, so handled
		// shortcuts keep the browser's own meaning from running.
		let _keys = window_event_listener(ev::keydown, move |event| {
			let action = shortcut_key(&event).and_then(|key| {
				state.with_untracked(|state| shortcut(state, key, event.ctrl_key()))
			});
			if let Some(action) = action {
				event.prevent_default();
				state.update(|state| state.apply(action));
			}
		});

		// Keyboard focus follows the interface: the prompt, or the palette's
		// search while it is open.
		Effect::new(move |_| {
			let target = state.with(Opencode::focus_target);
			let field = document().get_element_by_id(target).and_then(|element| {
				wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(element).ok()
			});
			if let Some(field) = field {
				let _ = field.focus();
			}
		});

		move || state.with(|state| leptoskit::web::render(state.render(&dispatch)))
	});
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
	eprintln!("Run `trunk serve` in examples/opencode to open the browser version.");
}
