//! The spec export as the CLI drives it: `export_if_requested` writes the
//! spec and exits the process, so these tests run it in a child process.
#![cfg(feature = "ssr")]

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

use leptoskit_rpc::spec;
use leptoskit_rpc::spec::SpecOptions;

/// Set in the child: the test body becomes the app's `main`.
const CHILD: &str = "LEPTOSKIT_RPC_EXPORT_CHILD";

/// A scratch directory unique to this process and `tag`.
fn scratch(tag: &str) -> PathBuf {
	let dir =
		std::env::temp_dir().join(format!("leptoskit_rpc_export_{tag}_{}", std::process::id()));
	std::fs::create_dir_all(&dir).expect("scratch");
	dir
}

/// Runs the test named `test` again in a child, as an app's `main` asked to
/// export its spec to `out`.
fn export_in_child(test: &str, out: &Path) -> Output {
	Command::new(std::env::current_exe().expect("the test binary"))
		.args(["--exact", test, "--nocapture", "--test-threads=1"])
		.env(CHILD, "1")
		.env(spec::SPEC_OUT_ENV, out)
		.output()
		.expect("run the child")
}

#[test]
fn a_requested_export_writes_the_spec_and_exits_cleanly() {
	if std::env::var_os(CHILD).is_some() {
		spec::export_if_requested();
		unreachable!("export_if_requested exits once an export is requested");
	}

	let dir = scratch("ok");
	let out = dir.join("spec.json");
	let child = export_in_child("a_requested_export_writes_the_spec_and_exits_cleanly", &out);
	let stderr = String::from_utf8_lossy(&child.stderr);
	assert!(child.status.success(), "{stderr}");

	assert!(
		stderr.contains("wrote the RPC specification to"),
		"{stderr}"
	);
	let written = std::fs::read_to_string(&out).expect("the spec file");
	assert!(spec::ApiSpec::from_json(&written).is_ok(), "{written}");
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_export_reports_the_error_and_exits_with_failure() {
	if std::env::var_os(CHILD).is_some() {
		spec::export_if_requested_with(&SpecOptions::new("Todos", "1.0.0"));
		unreachable!("export_if_requested_with exits once an export is requested");
	}

	// The output path is an existing directory, which cannot be written as a
	// file.
	let dir = scratch("fail");
	let child = export_in_child(
		"a_failed_export_reports_the_error_and_exits_with_failure",
		&dir,
	);
	let stderr = String::from_utf8_lossy(&child.stderr);
	assert_eq!(child.status.code(), Some(1), "{stderr}");
	assert!(
		stderr.contains("could not export the RPC specification"),
		"{stderr}"
	);

	assert!(
		stderr.contains("os error"),
		"the IO error that caused it is reported too: {stderr}"
	);
	let _ = std::fs::remove_dir_all(&dir);
}

/// Without the variable the server starts as usual: the call returns.
#[test]
fn an_unrequested_export_returns_to_main() {
	assert!(
		std::env::var_os(spec::SPEC_OUT_ENV).is_none(),
		"the test environment must not request an export"
	);
	spec::export_if_requested();
	spec::export_if_requested_with(&SpecOptions::new("Todos", "1.0.0"));
}

#[test]
fn writing_over_a_directory_names_the_path() {
	let dir = scratch("directory");
	let error = spec::write_to(&SpecOptions::new("Todos", "1.0.0"), &dir)
		.expect_err("a directory is not a file");
	assert!(
		std::error::Error::source(&error).is_some(),
		"the IO error is the source"
	);

	match error {
		spec::SpecError::Write { path, .. } => assert_eq!(path, dir),
		other => panic!("expected a write error, got {other}"),
	}

	let _ = std::fs::remove_dir_all(&dir);
}
