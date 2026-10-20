//! Snapshot integration tests: what the CLI generates — scaffolded
//! projects, help text, error messages, and code push artifacts — captured
//! with insta, so every change to generated output arrives as a reviewable
//! `.snap` diff instead of passing silently.
//!
//! Projects under test come from `tests/fixtures`. Output is normalized
//! before it is pinned (see [`normalize`]), so one snapshot set serves
//! every platform. Record new snapshots with `cargo insta test --accept`
//! (or `INSTA_UPDATE=always cargo test`) and accept them with `cargo insta
//! review`; plain `cargo test` fails on a missing or changed snapshot, so
//! CI cannot pass with an unreviewed output change.

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

/// Makes CLI output snapshot-stable across machines and platforms, with
/// plain string replacement rather than insta's regex filters, which a
/// backslash-laden Windows path would break.
///
/// The run-specific `root` becomes `[ROOT]` in both its canonicalized form
/// and its Windows verbatim-stripped twin, since code that joins onto the
/// working directory can print either; the path that follows each `[ROOT]`
/// gets `/` separators; and clap's `leptoskit.exe` usage name on Windows
/// loses its suffix.
fn normalize(text: &str, root: &Path) -> String {
	let root = root.to_string_lossy();
	let mut text = text.replace(root.as_ref(), "[ROOT]");
	let stripped = root.trim_start_matches(r"\\?\");
	if stripped != root {
		text = text.replace(stripped, "[ROOT]");
	}
	let mut normalized = String::with_capacity(text.len());
	let mut segments = text.split("[ROOT]");
	normalized.push_str(segments.next().unwrap_or_default());
	for segment in segments {
		let path_end = segment.find(char::is_whitespace).unwrap_or(segment.len());
		normalized.push_str("[ROOT]");
		normalized.push_str(&segment[..path_end].replace('\\', "/"));
		normalized.push_str(&segment[path_end..]);
	}
	normalized.replace("leptoskit.exe", "leptoskit")
}

/// `normalize` is exercised against Windows-shaped output here, since the
/// snapshots are recorded on Unix and only CI sees the real thing.
#[test]
fn normalize_makes_windows_output_match_unix() {
	let root = Path::new(r"\\?\C:\Users\runner\Temp\scratch_7");
	let output = "created \\\\?\\C:\\Users\\runner\\Temp\\scratch_7\\snapshotted (web \
	              project)\nnext: cd C:\\Users\\runner\\Temp\\scratch_7\\snapshotted && leptoskit \
	              build\nerror: could not parse \
	              C:\\Users\\runner\\Temp\\scratch_7\\dupe\\leptoskit.toml: bad\nUsage: \
	              leptoskit.exe <COMMAND>\n";
	assert_eq!(
		normalize(output, root),
		"created [ROOT]/snapshotted (web project)\nnext: cd [ROOT]/snapshotted && leptoskit \
		 build\nerror: could not parse [ROOT]/dupe/leptoskit.toml: bad\nUsage: leptoskit \
		 <COMMAND>\n"
	);
	assert_eq!(
		normalize("[ROOT]/already/unix", Path::new("/tmp/x")),
		"[ROOT]/already/unix",
		"Unix output passes through untouched"
	);
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
		&["generate", "--help"][..],
		&["generate", "app", "--help"][..],
		&["generate", "plugin", "--help"][..],
		&["native", "--help"][..],
		&["native", "prepare", "--help"][..],
		&["native", "build", "--help"][..],
		&["new", "--help"][..],
		&["dev", "--help"][..],
		&["build", "--help"][..],
		&["serve", "--help"][..],
		&["test", "--help"][..],
		&["test", "run", "--help"][..],
		&["test", "web", "--help"][..],
		&["codepush", "--help"][..],
		&["codepush", "release", "--help"][..],
		&["codepush", "publish", "--help"][..],
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
		insta::assert_snapshot!(name, normalize(&stdout(&output), &scratch.path));
	}
}

/// Every file a web scaffold writes, plus its output, pinned file by file.
#[test]
fn web_scaffold_is_pinned_file_by_file() {
	let scratch = Scratch::new("snap-web");
	let output = leptoskit(&scratch.path, &["new", "snapshotted"]);
	assert!(output.status.success(), "{}", stderr(&output));
	insta::assert_snapshot!(
		"new_web__stdout",
		normalize(&stdout(&output), &scratch.path)
	);
	let project = scratch.path.join("snapshotted");
	for file in new_project::template_files() {
		let contents = std::fs::read_to_string(project.join(file))
			.unwrap_or_else(|error| panic!("read {file}: {error}"));
		insta::assert_snapshot!(
			format!("new_web__{}", file.replace(['/', '\\'], "_")),
			contents
		);
	}
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
	insta::assert_snapshot!(
		"new_native__stdout",
		normalize(&stdout(&output), &scratch.path)
	);
	let project = scratch.path.join("snapshotted");
	let mut files = vec![leptoskit_config::CONFIG_FILE.to_owned()];
	files.extend(
		[
			"Cargo.toml",
			"hosts/ios/App.swift",
			"hosts/ios/Info.plist",
			"hosts/android/app/build.gradle.kts",
			"hosts/android/app/src/main/AndroidManifest.xml",
			"hosts/android/app/src/main/java/MainActivity.kt",
		]
		.map(str::to_owned),
	);
	for file in files {
		let contents = std::fs::read_to_string(project.join(&file))
			.unwrap_or_else(|error| panic!("read {file}: {error}"));
		insta::assert_snapshot!(
			format!("new_native__{}", file.replace(['/', '\\'], "_")),
			contents
		);
	}
}

/// The errors a broken project produces, so their wording is a contract.
#[test]
fn project_errors_are_pinned() {
	let malformed = fixture_project("malformed");
	let output = leptoskit(&malformed.path, &["build"]);
	assert!(!output.status.success());
	insta::assert_snapshot!(
		"errors__malformed_config",
		normalize(&stderr(&output), &malformed.path)
	);

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
	insta::assert_snapshot!(
		"errors__existing_project",
		normalize(&error.to_string(), &scratch.path)
	);

	let error = new_project::run(
		scratch.path.join("app").to_string_lossy().as_ref(),
		leptoskit_config::Platform::Web,
		Some("1 bad name"),
	)
	.expect_err("invalid name");
	insta::assert_snapshot!("errors__invalid_name", error.to_string());
}

/// Settings that mask every 64-hex-digit value as `label`: a publisher's
/// public key or a bundle hash. Both are run-specific — keys come out of
/// `keygen` fresh each run, and the bundle hash covers the creation
/// timestamp — so this is the one place a regex filter earns its keep.
fn with_hex_as(label: &str) -> insta::Settings {
	let mut settings = insta::Settings::clone_current();
	settings.add_filter("[0-9a-f]{64}", label);
	settings
}

/// Two signed releases from the codepush fixture: the keygen guidance, each
/// release's stdout, the store's tree, and each packed manifest — hashes,
/// sizes, versions, and the delta between them — with only the run-specific
/// values masked. A change in any file hash or size shows up here before it
/// reaches devices.
#[test]
fn codepush_artifacts_are_pinned() {
	let project = fixture_project("codepush");
	let keygen = leptoskit(
		&project.path,
		&["codepush", "keygen", "--out", "publisher.key"],
	);
	assert!(keygen.status.success(), "{}", stderr(&keygen));
	let release = ["codepush", "release", "--signing-key", "publisher.key"];
	let release_one = leptoskit(&project.path, &release);
	assert!(release_one.status.success(), "{}", stderr(&release_one));

	// A second, changed release on the same version: build 2 plus a delta.
	std::fs::write(project.path.join("dist/index.html"), "<p>again</p>").expect("index");
	let release_two = leptoskit(&project.path, &release);
	assert!(release_two.status.success(), "{}", stderr(&release_two));

	with_hex_as("[public key]").bind(|| {
		insta::assert_snapshot!("codepush__keygen_stdout", stdout(&keygen));
		insta::assert_snapshot!("codepush__release_one_stdout", stdout(&release_one));
		insta::assert_snapshot!("codepush__release_two_stdout", stdout(&release_two));
	});
	// A delta is named by the hash of the container it patches, which covers
	// that release's creation timestamp.
	with_hex_as("[base container hash]").bind(|| {
		insta::assert_snapshot!(
			"codepush__store_tree",
			file_tree(&project.path.join("releases"))
		);
	});

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

	// The offer a behind client sees.
	let behind = leptoskit(
		&project.path,
		&["codepush", "check", "--version", "0.1.0", "--build", "1"],
	);
	assert!(behind.status.success(), "{}", stderr(&behind));
	with_hex_as("[bundle hash]")
		.bind(|| insta::assert_snapshot!("codepush__check_stdout", stdout(&behind)));
}
