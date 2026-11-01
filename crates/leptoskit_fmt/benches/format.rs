//! Formatting speed on large files and on a whole workspace.
//!
//! `fixtures/dashboard.rs` is a module of components with large `view!`
//! blocks; each benchmark repeats it `copies` times, 160 to 250 lines a copy.
//!
//! - `formatted`: the steady state, formatting a file that already is.
//! - `unformatted`: the same file with every line's indentation stripped.
//! - `engine`: the formatted file with a script that echoes its input standing
//!   in for rustfmt, which measures this crate's own work plus starting one
//!   process.
//! - `workspace_cached`: a run over this repository where the cache says every
//!   file is formatted, as on a second `leptoskit format`.
//!
//! Run with `cargo bench -p leptoskit_fmt`.

use std::path::PathBuf;
use std::sync::LazyLock;

use divan::Bencher;
use leptoskit_fmt::Rustfmt;
use leptoskit_fmt::Settings;
use leptoskit_fmt::format_source;
use leptoskit_fmt::workspace;

fn main() {
	divan::main();
}

const COPIES: &[usize] = &[1, 4, 16];

static SETTINGS: LazyLock<Settings> = LazyLock::new(|| {
	Settings::for_path(&manifest_dir().join("benches/fixtures/dashboard.rs")).expect("settings")
});

/// Nightly rustfmt, the default, or the `PATH` rustfmt without it.
static RUSTFMT: LazyLock<Rustfmt> = LazyLock::new(|| {
	Rustfmt::locate("nightly")
		.or_else(|_| Rustfmt::locate(""))
		.expect("rustfmt")
});

fn manifest_dir() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The fixture repeated `copies` times and formatted.
fn formatted(copies: usize) -> String {
	let template = std::fs::read_to_string(manifest_dir().join("benches/fixtures/dashboard.rs"))
		.expect("fixture");
	let source: String = (0..copies)
		.map(|copy| template.replace("__N__", &copy.to_string()))
		.collect();
	format_source(&source, &SETTINGS, &RUSTFMT)
		.expect("format")
		.text
}

#[divan::bench(args = COPIES, sample_count = 20)]
fn formatted_file(bencher: Bencher<'_, '_>, copies: usize) {
	let source = formatted(copies);
	bencher
		.counter(divan::counter::BytesCount::of_str(&source))
		.bench(|| format_source(&source, &SETTINGS, &RUSTFMT).expect("format"));
}

#[divan::bench(args = COPIES, sample_count = 10)]
fn unformatted_file(bencher: Bencher<'_, '_>, copies: usize) {
	let mut source = String::new();

	for line in formatted(copies).lines() {
		source.push_str(line.trim_start());
		source.push('\n');
	}

	bencher
		.counter(divan::counter::BytesCount::of_str(&source))
		.bench(|| format_source(&source, &SETTINGS, &RUSTFMT).expect("format"));
}

#[divan::bench(args = COPIES, sample_count = 20)]
fn engine(bencher: Bencher<'_, '_>, copies: usize) {
	let source = formatted(copies);
	let cat = Rustfmt::at(pass_through());
	bencher
		.counter(divan::counter::BytesCount::of_str(&source))
		.bench(|| format_source(&source, &SETTINGS, &cat).expect("format"));
}

/// A stand-in for rustfmt that echoes its input, whatever its arguments.
fn pass_through() -> PathBuf {
	let path = std::env::temp_dir().join(format!("leptoskit-fmt-echo-{}", std::process::id()));
	std::fs::write(&path, "#!/bin/sh\nexec cat\n").expect("write script");
	let status = std::process::Command::new("chmod")
		.arg("+x")
		.arg(&path)
		.status()
		.expect("chmod");
	assert!(status.success(), "chmod failed");
	path
}

#[divan::bench(sample_count = 10)]
fn workspace_cached(bencher: Bencher<'_, '_>) {
	let root = manifest_dir().join("../..");
	// As the repository's own lint runs it: fixtures are unformatted on
	// purpose.
	let excludes = ["**/fixtures/**".to_owned(), "**/snapshots/**".to_owned()];
	let cache = std::env::temp_dir().join(format!("leptoskit-fmt-bench-{}", std::process::id()));
	let options = workspace::Options {
		check: true,
		toolchain: None,
		cache: Some(cache.clone()),
		concurrent: false,
	};
	// Warm the cache with every file that is already formatted.
	let files = workspace::collect_files(std::slice::from_ref(&root), &excludes).expect("files");
	drop(workspace::format_files(&files, &options));
	bencher.bench(|| {
		let files =
			workspace::collect_files(std::slice::from_ref(&root), &excludes).expect("files");
		workspace::format_files(&files, &options)
	});
	let _ = std::fs::remove_dir_all(cache);
}
