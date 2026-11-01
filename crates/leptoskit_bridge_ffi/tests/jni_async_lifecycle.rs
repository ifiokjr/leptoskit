//! Runs the production Kotlin/JNI callback adapter under JVM JNI checking.
#![cfg(all(feature = "app", any(target_os = "macos", target_os = "linux")))]

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

fn succeeds(command: &mut Command) {
	let output = command.output().expect("execute native toolchain command");
	assert!(
		output.status.success(),
		"{command:?}\nstdout:\n{}\nstderr:\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr),
	);
	assert!(
		!String::from_utf8_lossy(&output.stdout).contains("WARNING in native method"),
		"JVM reported a JNI contract violation"
	);
}

fn tool_home(name: &str) -> PathBuf {
	let path = std::env::var_os("PATH").expect("PATH is set");
	std::env::split_paths(&path)
		.map(|directory| directory.join(name))
		.find(|candidate| candidate.is_file())
		.unwrap_or_else(|| panic!("{name} must be installed"))
		.canonicalize()
		.unwrap()
		.parent()
		.unwrap()
		.parent()
		.unwrap()
		.to_owned()
}

fn retain_artifacts(directory: &Path) {
	let Some(destination) = std::env::var_os("LEPTOSKIT_JNI_TEST_ARTIFACT_DIR") else {
		return;
	};

	let destination = PathBuf::from(destination);
	std::fs::create_dir(&destination).expect("create fresh JNI artifact directory");

	for entry in std::fs::read_dir(directory).unwrap() {
		let path = entry.unwrap().path();
		std::fs::copy(&path, destination.join(path.file_name().unwrap())).unwrap();
	}
}

fn build_native_library(root: &Path, directory: &Path) {
	let jdk = tool_home("javac");
	let (platform, library, linking) = if cfg!(target_os = "macos") {
		("darwin", "libasync_host_test.dylib", "-dynamiclib")
	} else {
		("linux", "libasync_host_test.so", "-shared")
	};

	succeeds(
		Command::new("c++")
			.args([
				"-std=c++17",
				"-Wall",
				"-Wextra",
				"-Werror",
				"-fPIC",
				"-pthread",
			])
			.arg(linking)
			.arg("-I")
			.arg(jdk.join("include"))
			.arg("-I")
			.arg(jdk.join("include").join(platform))
			.arg(root.join("mobile/app.cpp"))
			.arg(root.join("async-host-tests/async.cpp"))
			.arg(root.join("async-host-tests/jni.cpp"))
			.arg("-o")
			.arg(directory.join(library)),
	);
}

fn build_kotlin_jar(root: &Path, directory: &Path, coroutines: &Path) -> PathBuf {
	let jar = directory.join("async.jar");
	succeeds(
		Command::new("kotlinc")
			.env("JAVA_OPTS", "-XX:ActiveProcessorCount=2 -Xmx512m")
			.arg(root.join("android/NativePlugin.kt"))
			.arg(root.join("mobile/NativeApp.kt"))
			.arg(root.join("mobile/NativeServiceOwner.kt"))
			.arg(root.join("mobile/NativeRustAsync.kt"))
			.arg(root.join("async-host-tests/JSONObject.kt"))
			.arg(root.join("async-host-tests/AsyncLifecycle.kt"))
			.arg("-cp")
			.arg(coroutines)
			.arg("-include-runtime")
			.arg("-d")
			.arg(&jar),
	);

	jar
}

#[test]
#[ignore = "run by test:plugins:jni, which provides the Kotlin/JDK and C++ toolchains; https://github.com/ifiokjr/leptoskit/issues/86"]
fn jni_async_callbacks_release_cancelled_capacity_after_cleanup() {
	let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
	let unique = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.unwrap()
		.as_nanos();
	let directory = std::env::temp_dir().join(format!(
		"leptoskit-jni-async-{}-{unique}",
		std::process::id()
	));
	std::fs::create_dir(&directory).unwrap();
	let coroutines = tool_home("kotlinc").join("lib/kotlinx-coroutines-core-jvm.jar");
	assert!(
		coroutines.is_file(),
		"Kotlin distribution includes coroutines"
	);
	build_native_library(&root, &directory);
	let jar = build_kotlin_jar(&root, &directory, &coroutines);

	succeeds(
		Command::new("java")
			.args(["-Xcheck:jni", "-XX:ActiveProcessorCount=2"])
			.arg(format!("-Djava.library.path={}", directory.display()))
			.arg("-cp")
			.arg(std::env::join_paths([jar, coroutines]).unwrap())
			.arg("dev.leptoskit.mobile.AsyncLifecycleKt"),
	);
	retain_artifacts(&directory);
	std::fs::remove_dir_all(directory).unwrap();
}
