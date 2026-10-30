//! Registers, queries and removes a real login entry through the bridge
//! adapter. On macOS and Linux the user's home and configuration directories
//! point at a temporary directory first, so the user's login items are never
//! touched. On Windows the entry uses a unique per-process identity under the
//! current user's `Run` key and is always removed before exit.
//!
//! This binary owns its entry point (`harness = false`) and runs its single
//! libtest-mimic trial on the main thread, so the environment is changed
//! before any other thread exists.
#![allow(unsafe_code)]

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error as BridgeError;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;
use leptoskit_plugin_autostart::Autostart;
use leptoskit_plugin_autostart::DISABLE;
use leptoskit_plugin_autostart::ENABLE;
use leptoskit_plugin_autostart::IS_REGISTERED;

fn main() {
	let mut arguments = libtest_mimic::Arguments::from_args();
	arguments.test_threads = Some(1);
	let trials = vec![libtest_mimic::Trial::test(
		"registers_queries_and_removes_a_real_login_entry",
		|| {
			registers_queries_and_removes_a_real_login_entry();
			Ok(())
		},
	)];
	libtest_mimic::run(&arguments, trials).exit();
}

fn registers_queries_and_removes_a_real_login_entry() {
	// Windows registers under the current user's `Run` key instead of a home
	// directory file.
	#[cfg(any(
		target_os = "macos",
		all(target_os = "linux", not(target_env = "ohos"))
	))]
	let home = tempfile::tempdir().unwrap();
	#[cfg(any(
		target_os = "macos",
		all(target_os = "linux", not(target_env = "ohos"))
	))]
	// SAFETY: No other thread exists yet, and nothing has read the environment.
	unsafe {
		std::env::set_var("HOME", home.path());
		std::env::set_var("XDG_CONFIG_HOME", home.path().join(".config"));
	}
	let identity = format!("dev.leptoskit.autostart-test-{}", std::process::id());
	let executable = std::env::current_exe().unwrap();
	let arguments = vec!["--hidden".to_owned(), "a \"quoted\" $value".to_owned()];
	let mut app = Autostart::new(identity.clone(), &executable, arguments.clone()).unwrap();
	assert_eq!(app.identity(), identity);
	assert_eq!(app.executable(), executable);
	assert_eq!(app.arguments(), arguments);
	assert_eq!(app.name(), "autostart");

	let registered = |app: &mut Autostart| app.invoke(IS_REGISTERED, Bytes::new()).unwrap();
	assert_eq!(registered(&mut app).as_ref(), [0]);
	let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
		assert_eq!(app.invoke(ENABLE, Bytes::new()), Ok(Bytes::new()));
		assert_eq!(app.invoke(ENABLE, Bytes::new()), Ok(Bytes::new()));
		assert_eq!(registered(&mut app).as_ref(), [1]);
		#[cfg(target_os = "macos")]
		assert!(
			home.path()
				.join(format!("Library/LaunchAgents/{identity}.plist"))
				.is_file()
		);
		#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
		assert!(
			home.path()
				.join(format!(".config/autostart/{identity}.desktop"))
				.is_file()
		);
		assert_eq!(
			app.invoke(ENABLE, Bytes::from_static(b"x")),
			Err(BridgeError::InvalidArgument)
		);
		assert_eq!(
			app.invoke(MethodId(9), Bytes::new()),
			Err(BridgeError::Unavailable)
		);
	}));
	// Remove the entry even when an assertion above failed.
	assert_eq!(app.invoke(DISABLE, Bytes::new()), Ok(Bytes::new()));
	assert_eq!(app.invoke(DISABLE, Bytes::new()), Ok(Bytes::new()));
	assert_eq!(registered(&mut app).as_ref(), [0]);
	if let Err(panic) = result {
		std::panic::resume_unwind(panic);
	}
}
