//! The binary itself: the summary reads in the language asked for, or the
//! system's, snapshots draw real AppKit views on macOS, and mistakes say what
//! to try.

use std::process::Command;
use std::process::Output;

fn postcard(args: &[&str], lang: &str) -> Output {
	Command::new(env!("CARGO_BIN_EXE_postcard_example"))
		.args(args)
		.env("LANG", lang)
		.output()
		.expect("the binary runs")
}

fn text(bytes: &[u8]) -> String {
	String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn the_summary_reads_in_the_language_asked_for() {
	let polish = postcard(&["summary", "pl"], "en_GB.UTF-8");
	assert!(polish.status.success(), "{}", text(&polish.stderr));
	assert!(text(&polish.stdout).contains("Pocztówka"));

	let arabic = postcard(&["summary", "ar"], "en_GB.UTF-8");
	assert!(text(&arabic.stdout).contains("بطاقة بريدية"));
}

#[test]
fn without_a_tag_the_summary_reads_in_the_systems_language() {
	let japanese = postcard(&["summary"], "ja_JP.UTF-8");
	assert!(japanese.status.success(), "{}", text(&japanese.stderr));
	assert!(text(&japanese.stdout).contains("絵はがき"));

	// A language the app does not have falls back to English.
	let german = postcard(&["summary"], "de_DE.UTF-8");
	assert!(text(&german.stdout).contains("\"Postcard\""));
}

#[test]
fn mistakes_fail_and_say_what_to_try() {
	let unknown = postcard(&["stamp"], "en");
	assert!(!unknown.status.success());
	assert!(text(&unknown.stderr).contains("unknown command `stamp`"));

	let pathless = postcard(&["snapshot"], "en");
	assert!(!pathless.status.success());
	assert!(text(&pathless.stderr).contains("snapshot needs a path"));
}

#[cfg(target_os = "macos")]
#[test]
fn a_snapshot_is_a_png_drawn_by_appkit() {
	let path = std::env::temp_dir().join(format!("postcard-{}.png", std::process::id()));
	let output = postcard(
		&["snapshot", path.to_str().expect("a UTF-8 path"), "ar"],
		"en",
	);
	assert!(output.status.success(), "{}", text(&output.stderr));
	let png = std::fs::read(&path).expect("the snapshot was written");
	std::fs::remove_file(&path).ok();
	assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
}

#[cfg(not(target_os = "macos"))]
#[test]
fn without_macos_the_window_is_the_summary_and_snapshots_refuse() {
	let output = postcard(&[], "pl_PL.UTF-8");
	assert!(output.status.success(), "{}", text(&output.stderr));
	assert!(text(&output.stdout).contains("Pocztówka"));
	assert!(text(&output.stderr).contains("windows open on macOS"));

	let snapshot = postcard(&["snapshot", "card.png"], "en");
	assert!(!snapshot.status.success());
	assert!(text(&snapshot.stderr).contains("need macOS"));
}
