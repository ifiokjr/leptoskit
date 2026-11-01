//! Executes the real Apple scheduler against controllably blocking native work.
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
fn queued_apple_worker_cancellation_does_not_invoke_plugin_or_free_capacity_early() {
	let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
	let unique = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.unwrap()
		.as_nanos();
	let directory = std::env::temp_dir().join(format!(
		"leptoskit-apple-workers-{}-{unique}",
		std::process::id()
	));
	std::fs::create_dir(&directory).unwrap();
	let object = directory.join("workers.o");
	let executable = directory.join("worker-lifecycle");
	succeeds(
		Command::new("xcrun")
			.args(["clang++", "-std=c++17", "-c"])
			.arg(root.join("apple-worker-tests/workers.cpp"))
			.arg("-o")
			.arg(&object),
	);
	succeeds(
		Command::new("xcrun")
			.args(["swiftc", "-swift-version", "6", "-parse-as-library"])
			.arg("-I")
			.arg(root.join("include"))
			.arg("-I")
			.arg(root.join("apple-worker-tests"))
			.arg(root.join("mobile/NativeRustWorker.swift"))
			.arg(root.join("apple/NativeJobBudget.swift"))
			.arg(root.join("apple-worker-tests/WorkerLifecycle.swift"))
			.arg(&object)
			.arg("-lc++")
			.arg("-o")
			.arg(&executable),
	);

	succeeds(&mut Command::new(executable));
	std::fs::remove_dir_all(directory).unwrap();
}
