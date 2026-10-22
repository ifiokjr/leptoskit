//! Call-syntax tests for the `nightly` feature.
//!
//! Compiled only when the `nightly` feature is enabled and the compiler is a
//! nightly build; on stable toolchains this file is an empty test crate.

#![cfg(all(feature = "nightly", rustc_nightly))]

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
fn maybe_rw_signal_reads_by_calling() {
	Owner::new().with(|| {
		let plain = MaybeRwSignal::Value(7_u32);
		assert_eq!(plain(), 7);

		let shared = RwSignal::new(1_u32);
		let reactive = MaybeRwSignal::Signal(shared);
		assert_eq!(reactive(), 1);
		shared.set(9);
		assert_eq!(reactive(), 9);
	});
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
		(toggle)();
		assert!(!value());
	});
}

#[test]
fn signals_compose_as_closures() {
	Owner::new().with(|| {
		let viewport = RwSignal::new(Viewport {
			width: 640,
			height: 480,
		});
		let size = use_window_size(viewport);
		let breakpoints = use_breakpoints([("small", 640_u32), ("large", 1024)].into(), size.width);
		let large: Signal<bool> = breakpoints.greater_or_equal(&"large").unwrap();

		// Signals satisfy `Fn` bounds, so they pass wherever a closure would.
		assert!(!evaluate(large));
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
