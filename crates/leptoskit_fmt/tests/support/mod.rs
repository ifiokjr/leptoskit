//! Formatting with the fixtures' settings: their `rustfmt.toml` holds only
//! stable options, and rustfmt is the one on the `PATH`, which rustup pins to
//! the repository's toolchain, so the output is the same everywhere.

#![allow(dead_code, reason = "each test file uses part of the support module")]

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::LazyLock;

use leptoskit_fmt::Output;
use leptoskit_fmt::Rustfmt;
use leptoskit_fmt::Settings;

pub static SETTINGS: LazyLock<Settings> = LazyLock::new(|| {
	Settings::for_path(&fixtures_dir().join("fixture.rs")).expect("fixture settings")
});

pub static RUSTFMT: LazyLock<Rustfmt> = LazyLock::new(|| Rustfmt::locate("").expect("rustfmt"));

pub fn fixtures_dir() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("tests")
		.join("fixtures")
}

pub fn format(source: &str) -> Output {
	format_with(source, &SETTINGS)
}

pub fn format_with(source: &str, settings: &Settings) -> Output {
	leptoskit_fmt::format_source(source, settings, &RUSTFMT).expect("format")
}

/// The formatted text, followed by any diagnostics.
pub fn describe(output: &Output) -> String {
	let mut description = output.text.clone();
	for diagnostic in &output.diagnostics {
		let _ = writeln!(description, "// diagnostic {diagnostic}");
	}
	description
}
