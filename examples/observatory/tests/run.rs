//! The binary itself: the summary prints the screen a screen reader walks,
//! snapshots draw real AppKit views on macOS, and mistakes say what to try.

use std::process::Command;
use std::process::Output;

fn observatory(args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_observatory_example"))
		.args(args)
		.output()
		.expect("the binary runs")
}

fn text(bytes: &[u8]) -> String {
	String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn the_summary_prints_the_accessibility_tree() {
	let output = observatory(&["summary"]);
	assert!(output.status.success(), "{}", text(&output.stderr));
	let tree = text(&output.stdout);
	assert!(tree.contains("heading(1) \"Observatory\""), "{tree}");
	assert!(tree.contains("columnheader \"Magnitude\""));
	assert!(tree.contains("status \"Plate 1 of 6\""));
	assert!(tree.contains("button \"Night 1\""));
}

#[test]
fn mistakes_fail_and_say_what_to_try() {
	let unknown = observatory(&["telescope"]);
	assert!(!unknown.status.success());
	assert!(text(&unknown.stderr).contains("unknown command `telescope`"));

	let pathless = observatory(&["snapshot"]);
	assert!(!pathless.status.success());
	assert!(text(&pathless.stderr).contains("snapshot needs a path"));
}

#[cfg(target_os = "macos")]
#[test]
fn a_snapshot_is_a_png_drawn_by_appkit() {
	let path = std::env::temp_dir().join(format!("observatory-{}.png", std::process::id()));
	let output = observatory(&["snapshot", path.to_str().expect("a UTF-8 path")]);
	assert!(output.status.success(), "{}", text(&output.stderr));
	let png = std::fs::read(&path).expect("the snapshot was written");
	std::fs::remove_file(&path).ok();
	assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
}

#[cfg(not(target_os = "macos"))]
#[test]
fn without_macos_the_window_is_the_summary_and_snapshots_refuse() {
	let output = observatory(&[]);
	assert!(output.status.success(), "{}", text(&output.stderr));
	assert!(text(&output.stdout).contains("heading(1) \"Observatory\""));
	assert!(text(&output.stderr).contains("windows open on macOS"));

	let snapshot = observatory(&["snapshot", "sky.png"]);
	assert!(!snapshot.status.success());
	assert!(text(&snapshot.stderr).contains("need macOS"));
}
