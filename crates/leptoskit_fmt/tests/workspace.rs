//! Formatting many files: what each run reports, and what it skips.

mod support;

use std::path::PathBuf;

use leptoskit_fmt::FormatError;
use leptoskit_fmt::workspace;
use leptoskit_fmt::workspace::Options;
use leptoskit_fmt::workspace::Status;

fn scratch(name: &str) -> PathBuf {
	let dir = std::env::temp_dir().join(format!(
		"leptoskit-fmt-workspace-{name}-{}",
		std::process::id()
	));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).expect("scratch dir");
	std::fs::write(
		dir.join("rustfmt.toml"),
		"edition = \"2024\"\nhard_tabs = true\n",
	)
	.expect("rustfmt.toml");
	dir
}

fn options(cache: Option<PathBuf>) -> Options {
	Options {
		check: false,
		// The rustfmt on the `PATH`, as rustup resolves it for the tests.
		toolchain: Some(String::new()),
		cache,
		concurrent: false,
	}
}

#[test]
fn unreadable_and_misconfigured_files_fail_on_their_own() {
	let root = scratch("failures");
	std::fs::write(root.join("good.rs"), "fn  good() {}\n").expect("write");
	std::fs::write(root.join("latin1.rs"), b"// caf\xe9\n").expect("write");
	std::fs::create_dir_all(root.join("app")).expect("app");
	std::fs::write(root.join("app/leptoskit.toml"), "[format]\nmacros = 3\n").expect("write");
	std::fs::write(root.join("app/lib.rs"), "fn app() {}\n").expect("write");
	std::fs::write(root.join("app/other.rs"), "fn other() {}\n").expect("write");

	let files = workspace::collect_files(std::slice::from_ref(&root), &[]).expect("files");
	assert_eq!(files.len(), 4);
	let report = workspace::format_files(&files, &options(None));
	let status = |name: &str| {
		&report
			.files
			.iter()
			.find(|file| file.path.ends_with(name))
			.expect("reported")
			.status
	};
	assert!(matches!(status("good.rs"), Status::Changed { .. }));
	assert!(matches!(
		status("latin1.rs"),
		Status::Failed(FormatError::NotUtf8 { .. })
	));
	assert!(matches!(
		status("app/lib.rs"),
		Status::Failed(FormatError::Config { .. })
	));
	assert!(matches!(
		status("app/other.rs"),
		Status::Failed(FormatError::Config { .. })
	));
	assert_eq!(report.failed().count(), 3);
	assert_eq!(
		std::fs::read_to_string(root.join("good.rs")).expect("read"),
		"fn good() {}\n"
	);
	std::fs::remove_dir_all(root).expect("clean up");
}

#[test]
fn the_cache_skips_files_left_formatted() {
	let root = scratch("cache");
	let cache = root.join("cache");
	std::fs::write(root.join("lib.rs"), "fn lib() {}\n").expect("write");
	let files = vec![root.join("lib.rs")];

	let first = workspace::format_files(&files, &options(Some(cache.clone())));
	assert!(matches!(first.files[0].status, Status::Unchanged));
	let second = workspace::format_files(&files, &options(Some(cache.clone())));
	assert!(matches!(second.files[0].status, Status::Cached));

	std::fs::write(root.join("lib.rs"), "fn  lib() {}\n").expect("write");
	let edited = workspace::format_files(&files, &options(Some(cache.clone())));
	assert!(matches!(edited.files[0].status, Status::Changed { .. }));
	let after = workspace::format_files(&files, &options(Some(cache)));
	assert!(
		matches!(after.files[0].status, Status::Cached),
		"a file it wrote is known formatted"
	);
	std::fs::remove_dir_all(root).expect("clean up");
}

#[test]
fn a_missing_toolchain_fails_every_file() {
	let root = scratch("toolchain");
	std::fs::write(root.join("lib.rs"), "fn lib() {}\n").expect("write");
	let options = Options {
		toolchain: Some("leptoskit-fmt-no-such-toolchain".to_owned()),
		..options(None)
	};
	let report = workspace::format_files(&[root.join("lib.rs")], &options);
	assert!(matches!(
		report.files[0].status,
		Status::Failed(FormatError::RustfmtMissing { .. })
	));
	std::fs::remove_dir_all(root).expect("clean up");
}

#[test]
fn invalid_paths_and_globs_are_errors() {
	let error = workspace::collect_files(&[PathBuf::from("/nonexistent/leptoskit")], &[])
		.expect_err("missing");
	assert!(matches!(error, FormatError::Io { .. }), "{error}");
	let root = scratch("glob");
	let error =
		workspace::collect_files(std::slice::from_ref(&root), &["[".to_owned()]).expect_err("glob");
	assert!(matches!(error, FormatError::Config { .. }), "{error}");
	std::fs::remove_dir_all(root).expect("clean up");
}

#[test]
fn the_cache_lives_in_the_target_directory() {
	let root = scratch("target");
	// cargo's own target directory wins when the environment names one.
	let target = std::env::var_os("CARGO_TARGET_DIR")
		.map(|target| PathBuf::from(target).join("leptoskit/format"));
	assert_eq!(workspace::default_cache_dir(&root), target);
	std::fs::write(root.join("Cargo.lock"), "").expect("write");
	std::fs::create_dir_all(root.join("src")).expect("src");
	let expected = target.unwrap_or_else(|| root.join("target/leptoskit/format"));
	assert_eq!(
		workspace::default_cache_dir(&root.join("src")),
		Some(expected)
	);
	std::fs::remove_dir_all(root).expect("clean up");
}
