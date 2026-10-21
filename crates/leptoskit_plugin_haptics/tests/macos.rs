//! AppKit feedback must be requested on the main thread, so this test owns the
//! process entry point instead of running on a libtest worker thread.

fn main() {
	let mut args = libtest_mimic::Arguments::from_args();
	// One thread runs every trial on the process's main thread.
	args.test_threads = Some(1);
	#[cfg(target_os = "macos")]
	let trials = macos::trials();
	#[cfg(not(target_os = "macos"))]
	let trials = Vec::new();
	libtest_mimic::run(&args, trials).exit();
}

#[cfg(target_os = "macos")]
mod macos {
	use leptoskit_bridge::Bytes;
	use leptoskit_bridge::Plugin;
	use leptoskit_plugin_haptics::Effect;
	use leptoskit_plugin_haptics::Haptics;
	use leptoskit_plugin_haptics::IMPACT;
	use leptoskit_plugin_haptics::Impact;
	use leptoskit_plugin_haptics::NOTIFICATION;
	use leptoskit_plugin_haptics::Notification;
	use leptoskit_plugin_haptics::SELECTION;
	use libtest_mimic::Trial;

	pub fn trials() -> Vec<Trial> {
		vec![Trial::test(
			"appkit_accepts_every_portable_effect_on_the_main_thread",
			|| {
				appkit_accepts_every_portable_effect_on_the_main_thread();
				Ok(())
			},
		)]
	}

	fn appkit_accepts_every_portable_effect_on_the_main_thread() {
		// Without a Force Touch trackpad AppKit accepts and ignores the
		// pattern, so success means the request was accepted, never
		// that hardware moved.
		for effect in [
			Effect::Selection,
			Effect::Impact(Impact::Heavy),
			Effect::Notification(Notification::Success),
		] {
			assert_eq!(leptoskit_plugin_haptics::perform(effect), Ok(()));
		}
		let mut haptics = Haptics;
		assert_eq!(haptics.invoke(SELECTION, Bytes::new()), Ok(Bytes::new()));
		assert_eq!(
			haptics.invoke(IMPACT, Bytes::from(1_u32.to_le_bytes().to_vec())),
			Ok(Bytes::new())
		);
		assert_eq!(
			haptics.invoke(NOTIFICATION, Bytes::from(2_u32.to_le_bytes().to_vec())),
			Ok(Bytes::new())
		);
	}
}
