//! Signals read and write by being called on every toolchain: leptos 0.9
//! derefs each of its signals to a `dyn Fn`, and `MaybeRwSignal` does the same.

use leptoskit_use::MaybeRwSignal;
use leptoskit_use::UseToggleReturn;
use leptoskit_use::Viewport;
use leptoskit_use::use_breakpoints;
use leptoskit_use::use_toggle;
use leptoskit_use::use_window_size;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;
use reactive_graph::signal::RwSignal;
use reactive_graph::wrappers::read::Signal;

fn evaluate(callback: impl Fn() -> bool) -> bool {
	callback()
}

#[test]
fn hook_results_read_and_write_by_calling() {
	Owner::new().with(|| {
		let UseToggleReturn {
			value,
			set_value,
			toggle,
		} = use_toggle(false);
		assert!(!value());
		set_value(true);
		assert!(value());
		toggle();
		assert!(!value());
	});
}

#[test]
fn signals_pass_as_closures_once_dereferenced() {
	Owner::new().with(|| {
		let viewport = RwSignal::new(Viewport {
			width: 640,
			height: 480,
		});
		let size = use_window_size(viewport);
		let breakpoints = use_breakpoints([("small", 640_u32), ("large", 1024)].into(), size.width);
		let large: Signal<bool> = breakpoints.greater_or_equal(&"large").unwrap();

		// A signal is not itself a closure, but it derefs to one.
		assert!(!evaluate(&*large));
		assert!(evaluate(move || !large()));

		let derived = Signal::derive(move || (size.height)() > 500);
		assert!(!derived());
		viewport.set(Viewport {
			width: 1280,
			height: 768,
		});
		assert!(large());
		assert!(derived());
	});
}

#[test]
fn maybe_rw_signals_read_by_calling() {
	Owner::new().with(|| {
		let plain = MaybeRwSignal::Value(String::from("initial"));
		assert_eq!(plain(), "initial");
		assert_eq!(plain.get(), "initial");
		let named = MaybeRwSignal::Signal(RwSignal::new(String::from("shared")));
		assert_eq!(named(), "shared");

		let shared = RwSignal::new(false);
		let reactive = MaybeRwSignal::Signal(shared);
		assert!(!reactive());
		shared.set(true);
		assert!(reactive());
		assert!(evaluate(&*reactive));
		assert!(evaluate(&*MaybeRwSignal::Value(true)));
	});
}
