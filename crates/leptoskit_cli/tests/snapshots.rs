//! Snapshot integration tests: what the CLI generates — scaffolded
//! projects, help text, error messages, and code push artifacts — captured
//! with insta, so every change to generated output arrives as a reviewable
//! `.snap` diff instead of passing silently.
//!
//! Projects under test come from `tests/fixtures`. Record new snapshots
//! with `cargo insta test --accept` (or `INSTA_UPDATE=always cargo test`)
//! and accept them with `cargo insta review`; plain `cargo test` fails on
//! a missing or changed snapshot, so CI cannot pass with an unreviewed
//! output change.

use std::path::Path;
use std::process::Command;
use std::process::Output;

use leptoskit_cli::new_project;
use leptoskit_codepush::ReleaseStore;

mod support;

use support::Scratch;
use support::fixture_project;

/// Runs the built `leptoskit` binary in `dir`.
fn leptoskit(dir: &Path, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args(args)
		.current_dir(dir)
		.output()
		.expect("run leptoskit")
}

fn stderr(output: &Output) -> String {
	String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
	String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Snapshot settings that stand in for the run-specific `root` directory,
/// so snapshots stay identical across machines and runs. Both the
/// canonicalized form and its Windows verbatim-stripped twin are filtered:
/// code that joins onto the process working directory can print either.
fn settings_for(root: &Path) -> insta::Settings {
	let mut settings = insta::Settings::clone_current();
	let root = root.to_string_lossy();
	settings.add_filter(root.as_ref(), "[ROOT]");
	let stripped = root.trim_start_matches(r"\\?\");
	if stripped != root {
		settings.add_filter(stripped, "[ROOT]");
	}
	settings
}

/// Every file under `dir`, one `/`-separated relative path per line in
/// sorted order: the shape of a generated tree, without the bytes.
fn file_tree(dir: &Path) -> String {
	let mut paths: Vec<String> = walkdir::WalkDir::new(dir)
		.into_iter()
		.map(|entry| entry.expect("walk").into_path())
		.filter(|path| path.is_file())
		.map(|path| {
			path.strip_prefix(dir)
				.expect("walked paths live under the root")
				.to_string_lossy()
				.replace('\\', "/")
		})
		.collect();
	paths.sort();
	paths.join("\n") + "\n"
}

/// The help of the root command and every subcommand, snapshotted through
/// the real binary so flags, defaults, and descriptions are pinned.
#[test]
fn help_text_is_pinned() {
	let scratch = Scratch::new("help");
	for args in [
		&["--help"][..],
		&["new", "--help"][..],
		&["dev", "--help"][..],
		&["build", "--help"][..],
		&["serve", "--help"][..],
		&["codepush", "--help"][..],
		&["codepush", "release", "--help"][..],
		&["codepush", "serve", "--help"][..],
		&["codepush", "check", "--help"][..],
		&["doctor", "--help"][..],
		&["completions", "--help"][..],
	] {
		let output = leptoskit(&scratch.path, args);
		assert!(
			output.status.success(),
			"{} --help failed: {}",
			args.join(" "),
			stderr(&output)
		);
		let name = if args.len() == 1 {
			"help__root".to_owned()
		} else {
			format!("help__{}", args[..args.len() - 1].join("-"))
		};
		insta::assert_snapshot!(name, stdout(&output));
	}
}

/// Every file a web scaffold writes, plus its output, pinned file by file.
#[test]
fn web_scaffold_is_pinned_file_by_file() {
	let scratch = Scratch::new("snap-web");
	let output = leptoskit(&scratch.path, &["new", "snapshotted"]);
	assert!(output.status.success(), "{}", stderr(&output));
	settings_for(&scratch.path).bind(|| {
		insta::assert_snapshot!("new_web__stdout", stdout(&output));
		let project = scratch.path.join("snapshotted");
		for file in new_project::template_files() {
			let contents = std::fs::read_to_string(project.join(file))
				.unwrap_or_else(|error| panic!("read {file}: {error}"));
			insta::assert_snapshot!(
				format!("new_web__{}", file.replace(['/', '\\'], "_")),
				contents
			);
		}
	});
}

/// The native-only scaffold surface: the config that carries the native
/// platform and package id, and the Apple host files.
#[test]
fn native_scaffold_is_pinned() {
	let scratch = Scratch::new("snap-native");
	let output = leptoskit(
		&scratch.path,
		&["new", "snapshotted", "--platform", "native"],
	);
	assert!(output.status.success(), "{}", stderr(&output));
	settings_for(&scratch.path).bind(|| {
		insta::assert_snapshot!("new_native__stdout", stdout(&output));
		let project = scratch.path.join("snapshotted");
		let mut files = vec![leptoskit_config::CONFIG_FILE.to_owned()];
		files.extend(new_project::native_template_files("snapshotted"));
		for file in files {
			let contents = std::fs::read_to_string(project.join(&file))
				.unwrap_or_else(|error| panic!("read {file}: {error}"));
			insta::assert_snapshot!(
				format!("new_native__{}", file.replace(['/', '\\'], "_")),
				contents
			);
		}
	});
}

/// The errors a broken project produces, so their wording is a contract.
#[test]
fn project_errors_are_pinned() {
	let malformed = fixture_project("malformed");
	let output = leptoskit(&malformed.path, &["build"]);
	assert!(!output.status.success());
	settings_for(&malformed.path)
		.bind(|| insta::assert_snapshot!("errors__malformed_config", stderr(&output)));

	let unknown = fixture_project("unknown_plugin");
	let output = leptoskit(&unknown.path, &["build"]);
	assert!(!output.status.success());
	insta::assert_snapshot!("errors__unknown_plugin", stderr(&output));
}

/// The scaffold's own refusals: existing projects and invalid names.
#[test]
fn scaffold_errors_are_pinned() {
	let scratch = Scratch::new("snap-scaffold-errors");
	let project = scratch.path.join("dupe");
	new_project::run(
		project.to_string_lossy().as_ref(),
		leptoskit_config::Platform::Web,
		None,
	)
	.expect("first scaffold");
	let error = new_project::run(
		project.to_string_lossy().as_ref(),
		leptoskit_config::Platform::Web,
		None,
	)
	.expect_err("second scaffold");
	settings_for(&scratch.path)
		.bind(|| insta::assert_snapshot!("errors__existing_project", error.to_string()));

	let error = new_project::run(
		scratch.path.join("app").to_string_lossy().as_ref(),
		leptoskit_config::Platform::Web,
		Some("1 bad name"),
	)
	.expect_err("invalid name");
	insta::assert_snapshot!("errors__invalid_name", error.to_string());
}

/// Two releases from the codepush fixture: the store's tree, each release's
/// stdout, and each packed manifest — hashes, sizes, versions, and the
/// delta between them — with only the creation timestamp redacted. A change
/// in any hash or size shows up here before it reaches devices.
#[test]
fn codepush_artifacts_are_pinned() {
	let project = fixture_project("codepush");
	let release_one = leptoskit(&project.path, &["codepush", "release"]);
	assert!(release_one.status.success(), "{}", stderr(&release_one));

	// A second, changed release on the same version: build 2 plus a delta.
	std::fs::write(project.path.join("dist/index.html"), "<p>again</p>").expect("index");
	let release_two = leptoskit(&project.path, &["codepush", "release"]);
	assert!(release_two.status.success(), "{}", stderr(&release_two));

	settings_for(&project.path).bind(|| {
		insta::assert_snapshot!("codepush__release_one_stdout", stdout(&release_one));
		insta::assert_snapshot!("codepush__release_two_stdout", stdout(&release_two));
		insta::assert_snapshot!(
			"codepush__store_tree",
			file_tree(&project.path.join("releases"))
		);

		let store = ReleaseStore::open(project.path.join("releases")).expect("open store");
		let records = store.list("pushed", "production").expect("list releases");
		assert_eq!(records.len(), 2, "both releases live in the store");
		for record in &records {
			let name = format!(
				"codepush__manifest_{}+{}",
				record.manifest.version, record.manifest.build
			);
			insta::assert_json_snapshot!(
				name,
				serde_json::to_value(&record.manifest).expect("manifest"),
				{".created_unix" => "[creation timestamp]"}
			);
		}

		// The offer a behind client sees; the bundle hash covers the
		// timestamp, so it is filtered rather than pinned.
		let mut settings = settings_for(&project.path);
		settings.add_filter("[0-9a-f]{64}", "[bundle hash]");
		settings.bind(|| {
			let behind = leptoskit(
				&project.path,
				&["codepush", "check", "--version", "0.1.0", "--build", "1"],
			);
			assert!(behind.status.success(), "{}", stderr(&behind));
			insta::assert_snapshot!("codepush__check_stdout", stdout(&behind));
		});
	});
}
