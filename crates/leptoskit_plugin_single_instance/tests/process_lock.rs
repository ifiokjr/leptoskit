#![cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]

use leptoskit_plugin_single_instance::InstanceGuard;

#[test]
fn process_probe() {
	let Some(directory) = std::env::var_os("LEPTOSKIT_INSTANCE_TEST_DIR") else {
		return;
	};
	let acquired = InstanceGuard::acquire(
		std::path::Path::new(&directory),
		"dev.leptoskit.process-test",
	)
	.unwrap();
	let expected = std::env::var("LEPTOSKIT_INSTANCE_TEST_EXPECTED").unwrap() == "1";
	assert_eq!(acquired.is_some(), expected);
}

#[test]
fn other_process_is_excluded_until_owner_exits() {
	let directory = tempfile::tempdir().unwrap();
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
	}
	let guard = InstanceGuard::acquire(directory.path(), "dev.leptoskit.process-test")
		.unwrap()
		.unwrap();
	let probe = |expected| {
		std::process::Command::new(std::env::current_exe().unwrap())
			.args(["--exact", "process_probe"])
			.env("LEPTOSKIT_INSTANCE_TEST_DIR", directory.path())
			.env("LEPTOSKIT_INSTANCE_TEST_EXPECTED", expected)
			.output()
			.unwrap()
	};
	let blocked = probe("0");
	assert!(
		blocked.status.success(),
		"{}",
		String::from_utf8_lossy(&blocked.stderr)
	);
	drop(guard);
	let available = probe("1");
	assert!(
		available.status.success(),
		"{}",
		String::from_utf8_lossy(&available.stderr)
	);
}
