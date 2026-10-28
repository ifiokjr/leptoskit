//! Main-thread native Button controls, with an explicit arbitrary-thread
//! measurement control.

#[cfg(target_os = "macos")]
#[path = "button_metrics/mod.rs"]
mod cases;

fn main() {
	let mut args = libtest_mimic::Arguments::from_args();
	args.test_threads = Some(1);
	#[cfg(target_os = "macos")]
	let trials = vec![
		trial("button_updates_remeasure_the_same_node", cases::updates),
		trial(
			"button_metrics_match_native_controls",
			cases::native_controls,
		),
		trial(
			"button_measurement_works_off_main_thread",
			cases::off_main_thread,
		),
	];
	#[cfg(not(target_os = "macos"))]
	let trials = Vec::new();
	libtest_mimic::run(&args, trials).exit();
}

#[cfg(target_os = "macos")]
fn trial(name: &str, body: fn()) -> libtest_mimic::Trial {
	libtest_mimic::Trial::test(name, move || {
		body();
		Ok(())
	})
}
