#![cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]

use std::cell::Cell;
use std::rc::Rc;

use leptoskit_use::web::use_event_listener;
use leptoskit_use::web::use_media_query;
use leptoskit_use::web::use_preferred_dark;
use leptoskit_use::web::use_preferred_reduced_motion;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::Get;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::CustomEvent;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn media_queries_report_current_matches_and_reject_invalid_queries() {
	let owner = Owner::new();
	owner.with(|| {
		assert!(use_media_query("(min-width: 1px)").get());
		assert!(!use_media_query("(min-width: 99999999px)").get());
		assert!(!use_media_query("not a media query").get());
	});
	owner.cleanup();
}

#[wasm_bindgen_test]
fn preferred_hooks_mirror_the_browser_settings() {
	let owner = Owner::new();
	owner.with(|| {
		let window = web_sys::window().unwrap();
		let dark = use_preferred_dark().get();
		let reduced = use_preferred_reduced_motion().get();
		assert_eq!(
			dark,
			window
				.match_media("(prefers-color-scheme: dark)")
				.unwrap()
				.unwrap()
				.matches()
		);
		assert_eq!(
			reduced,
			window
				.match_media("(prefers-reduced-motion: reduce)")
				.unwrap()
				.unwrap()
				.matches()
		);
	});
	owner.cleanup();
}

#[wasm_bindgen_test]
fn event_listeners_dispatch_until_stopped_and_on_owner_cleanup() {
	let owner = Owner::new();
	let target = web_sys::window().unwrap().document().unwrap();
	let dispatch = {
		let target = target.clone();
		move || {
			target
				.dispatch_event(&CustomEvent::new("lk-test").unwrap())
				.unwrap();
		}
	};
	let owner_events = Owner::new();
	let count = Rc::new(Cell::new(0_u32));
	let seen = Rc::clone(&count);
	let stop = owner_events.with(|| {
		let event_target: web_sys::EventTarget = target.clone().unchecked_into();
		use_event_listener(&event_target, "lk-test", move |event: CustomEvent| {
			seen.set(seen.get() + 1);
			assert_eq!(event.type_(), "lk-test");
		})
	});
	dispatch();
	assert_eq!(count.get(), 1);
	(stop)();
	dispatch();
	assert_eq!(count.get(), 1);
	owner_events.cleanup();
	dispatch();
	assert_eq!(count.get(), 1);
	owner.cleanup();
}
