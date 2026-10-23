//! Executes the real Swift continuation adapter against controllable callbacks.
#![cfg(all(feature = "app", target_os = "macos"))]

use std::path::PathBuf;
use std::process::Command;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

fn succeeds(command: &mut Command) {
	let output = command.output().expect("execute Apple toolchain command");
	assert!(
		output.status.success(),
		"{command:?}\nstdout:\n{}\nstderr:\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr),
	);
}

#[test]
fn apple_async_callbacks_retain_cancelled_jobs_until_cleanup() {
	let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
	let unique = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.unwrap()
		.as_nanos();
	let directory = std::env::temp_dir().join(format!(
		"leptoskit-apple-async-{}-{unique}",
		std::process::id()
	));
	std::fs::create_dir(&directory).unwrap();
	let object = directory.join("async.o");
	let executable = directory.join("async-lifecycle");
	succeeds(
		Command::new("xcrun")
			.arg("clang++")
			.args(["-std=c++17", "-Wall", "-Wextra"])
			.args(["-Werror", "-c"])
			.arg(root.join("async-host-tests/async.cpp"))
			.arg("-o")
			.arg(&object),
	);
	succeeds(
		Command::new("xcrun")
			.args(["swiftc", "-swift-version", "6", "-parse-as-library"])
			.arg("-I")
			.arg(root.join("include"))
			.arg("-I")
			.arg(root.join("async-host-tests"))
			.arg(root.join("mobile/NativeRustWorker.swift"))
			.arg(root.join("mobile/NativeRustAsync.swift"))
			.arg(root.join("apple/NativeJobBudget.swift"))
			.arg(root.join("async-host-tests/AsyncLifecycle.swift"))
			.arg(&object)
			.arg("-lc++")
			.arg("-o")
			.arg(&executable),
	);

	succeeds(&mut Command::new(executable));
	std::fs::remove_dir_all(directory).unwrap();
}
