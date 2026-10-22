//! Formatting is idempotent over this repository's own sources: every Rust
//! file, formatted twice, comes out the same both times.

mod support;

use std::path::PathBuf;

use leptoskit_fmt::Settings;
use leptoskit_fmt::format_source;
use leptoskit_fmt::workspace::collect_files;
use rayon::prelude::*;

#[test]
fn formatting_the_repository_twice_changes_nothing() {
	let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
	let files: Vec<PathBuf> = collect_files(&[root], &[])
		.expect("walk the repository")
		.into_iter()
		.filter(|path| {
			!path
				.components()
				.any(|part| matches!(part.as_os_str().to_str(), Some("fixtures" | "snapshots")))
		})
		.collect();
	assert!(files.len() > 100, "found only {} files", files.len());
	let unstable: Vec<String> = files
		.par_iter()
		.filter_map(|path| {
			let source = std::fs::read_to_string(path).ok()?;
			let settings = Settings::for_path(path).expect("settings");
			let once = format_source(&source, &settings, &support::RUSTFMT).ok()?;
			let twice =
				format_source(&once.text, &settings, &support::RUSTFMT).expect("formats again");
			(twice.text != once.text).then(|| path.display().to_string())
		})
		.collect();
	assert!(
		unstable.is_empty(),
		"formatting twice changed: {unstable:#?}"
	);
}
