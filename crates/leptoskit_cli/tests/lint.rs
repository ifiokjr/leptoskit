//! `leptoskit lint` over scratch projects. Where clippy runs, a stand-in
//! `cargo` prints the JSON messages a real run would, so each test decides
//! what clippy reports without compiling anything.

use std::path::Path;
use std::process::Command;
use std::process::Output;

use tempfile::TempDir;

/// A builder tree, which the default rules warn about.
const BUILDER_TREE: &str =
	"fn screen() -> Element {\n\tView().class(\"p-4\").child(Text(\"Hi\"))\n}\n";

/// A project with `config` as its `leptoskit.toml` and `source` as
/// `src/lib.rs`.
fn project(config: &str, source: &str) -> TempDir {
	let project = tempfile::tempdir().expect("scratch project");
	std::fs::write(project.path().join("leptoskit.toml"), config).expect("write config");
	std::fs::create_dir(project.path().join("src")).expect("create src");
	std::fs::write(project.path().join("src/lib.rs"), source).expect("write source");
	project
}

/// Runs `leptoskit lint` in `dir`, with `cargo` standing in for cargo.
fn lint(dir: &Path, args: &[&str], cargo: &Path) -> Output {
	Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.arg("lint")
		.args(args)
		.current_dir(dir)
		.env("CARGO", cargo)
		.output()
		.expect("run leptoskit lint")
}

fn text(bytes: &[u8]) -> String {
	anstream::adapter::strip_str(&String::from_utf8_lossy(bytes)).to_string()
}

#[test]
fn clean_projects_pass_and_say_clippy_was_skipped() {
	let project = project("", "fn helper() -> u32 {\n\t1\n}\n");
	let output = lint(
		project.path(),
		&["--no-clippy"],
		Path::new("cargo-is-not-run"),
	);
	let stdout = text(&output.stdout);
	assert!(output.status.success(), "{stdout}");
	assert!(stdout.contains("1 file · no problems"), "{stdout}");
	assert!(stdout.contains("skipped with --no-clippy"), "{stdout}");
}

#[test]
fn the_configuration_can_turn_clippy_off() {
	let project = project("[lint]\nclippy = false\n", BUILDER_TREE);
	let output = lint(project.path(), &[], Path::new("cargo-is-not-run"));
	let stdout = text(&output.stdout);
	assert!(output.status.success(), "{stdout}");
	assert!(
		stdout.contains("warning[leptoskit::markup_style]"),
		"{stdout}"
	);
	assert!(stdout.contains("1 file · 1 warning"), "{stdout}");
	assert!(
		stdout.contains("skipped by `[lint] clippy = false`"),
		"{stdout}"
	);
}

#[test]
fn paths_narrow_the_rules_and_unparsed_files_are_counted() {
	let project = project("", BUILDER_TREE);
	std::fs::write(project.path().join("src/broken.rs"), "fn broken( {").expect("write source");
	let output = lint(
		project.path(),
		&["--no-clippy", "src/broken.rs"],
		Path::new("cargo-is-not-run"),
	);
	let stdout = text(&output.stdout);
	assert!(output.status.success(), "{stdout}");
	assert!(
		stdout.contains("1 file · no problems · 1 file not parsed"),
		"{stdout}"
	);
}

#[test]
fn rule_errors_fail_the_run() {
	let project = project("[lint.rules]\nmarkup_style = \"deny\"\n", BUILDER_TREE);
	let output = lint(
		project.path(),
		&["--no-clippy"],
		Path::new("cargo-is-not-run"),
	);
	let stdout = text(&output.stdout);
	assert!(!output.status.success(), "{stdout}");
	assert!(
		stdout.contains("error[leptoskit::markup_style]"),
		"{stdout}"
	);
	assert!(
		text(&output.stderr).contains("lint found problems to fix"),
		"{}",
		text(&output.stderr)
	);
}

#[test]
fn an_invalid_lint_table_fails_before_anything_runs() {
	let project = project("[lint]\nmarkup = \"html\"\n", BUILDER_TREE);
	let output = lint(
		project.path(),
		&["--no-clippy"],
		Path::new("cargo-is-not-run"),
	);
	assert!(!output.status.success());
	assert!(
		text(&output.stderr).contains("leptoskit.toml"),
		"{}",
		text(&output.stderr)
	);
}

#[test]
fn a_cargo_that_cannot_start_is_reported() {
	let project = project("", BUILDER_TREE);
	let output = lint(project.path(), &[], &project.path().join("no-such-cargo"));
	let stderr = text(&output.stderr);
	assert!(!output.status.success(), "{stderr}");
	assert!(stderr.contains("start `cargo clippy`"), "{stderr}");
}

/// The stand-in `cargo` is a shell script; the lines it covers run on the
/// Linux coverage job, which is where they matter.
#[cfg(unix)]
mod clippy {
	use std::os::unix::fs::PermissionsExt as _;
	use std::path::Path;
	use std::path::PathBuf;

	use super::BUILDER_TREE;
	use super::lint;
	use super::project;
	use super::text;

	const ARTIFACT: &str = r#"{"reason":"compiler-artifact","target":{"name":"demo"}}"#;
	const WARNING: &str = r#"{"reason":"compiler-message","message":{"level":"warning","rendered":"warning: unused variable `x`\n"}}"#;
	const NOTE: &str = r#"{"reason":"compiler-message","message":{"level":"note","rendered":"note: not counted"}}"#;
	const ERROR: &str = r#"{"reason":"compiler-message","message":{"level":"error","rendered":"error: mismatched types\n"}}"#;

	/// Writes a `cargo` into `dir` that records its arguments in `dir/args`,
	/// prints `stdout` and `stderr`, and exits with `code`.
	fn cargo(dir: &Path, stdout: &[&str], stderr: &str, code: i32) -> PathBuf {
		let path = dir.join("cargo");
		let args = dir.join("args");
		let script = format!(
			"#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ncat <<'EOF'\n{}\nEOF\nprintf '%s' \
			 '{stderr}' >&2\nexit {code}\n",
			args.display(),
			stdout.join("\n"),
		);
		std::fs::write(&path, script).expect("write cargo");
		std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
		path
	}

	fn arguments(dir: &Path) -> Vec<String> {
		std::fs::read_to_string(dir.join("args"))
			.expect("cargo recorded its arguments")
			.lines()
			.map(str::to_owned)
			.collect()
	}

	#[test]
	fn rule_findings_and_clippy_diagnostics_are_reported_together() {
		let project = project("", BUILDER_TREE);
		let stand_in = tempfile::tempdir().expect("cargo directory");
		let cargo = cargo(
			stand_in.path(),
			&[
				ARTIFACT,
				"Compiling demo",
				WARNING,
				WARNING,
				NOTE,
				ERROR,
				r#"{"reason":"build-finished","success":false}"#,
			],
			"",
			101,
		);
		let output = lint(project.path(), &["--", "-W", "clippy::nursery"], &cargo);
		let stdout = text(&output.stdout);
		assert!(!output.status.success(), "{stdout}");
		assert!(
			stdout.contains("warning[leptoskit::markup_style]"),
			"{stdout}"
		);
		// The same diagnostic from a library and its tests counts once.
		assert_eq!(stdout.matches("unused variable").count(), 1, "{stdout}");

		assert!(stdout.contains("error: mismatched types"), "{stdout}");
		assert!(!stdout.contains("not counted"), "{stdout}");
		assert!(stdout.contains("1 error · 1 warning"), "{stdout}");

		let arguments = arguments(stand_in.path());
		assert_eq!(arguments[0], "clippy");
		assert!(arguments.contains(&"--workspace".to_owned()));
		assert!(
			arguments.ends_with(&[
				"--".to_owned(),
				"-W".to_owned(),
				"clippy::nursery".to_owned()
			]),
			"{arguments:?}"
		);
	}

	#[test]
	fn clippy_warnings_pass_unless_warnings_are_denied() {
		let project = project("", "fn helper() -> u32 {\n\t1\n}\n");
		let stand_in = tempfile::tempdir().expect("cargo directory");
		let cargo = cargo(stand_in.path(), &[WARNING], "", 0);

		let output = lint(project.path(), &[], &cargo);
		let stdout = text(&output.stdout);
		assert!(output.status.success(), "{stdout}");
		assert!(stdout.contains("1 warning"), "{stdout}");

		let output = lint(project.path(), &["--deny-warnings"], &cargo);
		let stdout = text(&output.stdout);
		assert!(!output.status.success(), "{stdout}");
		// Through cargo's configuration, so cargo's own lints fail the run too.
		let arguments = arguments(stand_in.path());
		assert!(
			arguments.ends_with(&[
				"--config".to_owned(),
				"build.warnings=\"deny\"".to_owned(),
				"--".to_owned(),
			]),
			"{arguments:?}"
		);
	}

	#[test]
	fn cargo_failing_before_clippy_shows_why() {
		let project = project("", "fn helper() -> u32 {\n\t1\n}\n");
		let stand_in = tempfile::tempdir().expect("cargo directory");
		let cargo = cargo(stand_in.path(), &[], "error: failed to parse manifest", 101);
		let output = lint(project.path(), &[], &cargo);
		let stdout = text(&output.stdout);
		assert!(!output.status.success(), "{stdout}");
		assert!(
			stdout.contains("error: failed to parse manifest"),
			"{stdout}"
		);
		assert!(
			stdout.contains("cargo failed before clippy could check"),
			"{stdout}"
		);
	}
}
