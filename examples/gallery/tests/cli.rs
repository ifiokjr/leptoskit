//! The `gallery` binary's commands that need no window.

use std::process::Command;
use std::process::Output;

use gallery::SCREENS;

fn gallery(args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_gallery"))
		.args(args)
		.output()
		.expect("the gallery binary runs")
}

#[test]
fn list_prints_every_screen_with_its_viewport() {
	let output = gallery(&["list"]);
	assert!(output.status.success());
	let stdout = String::from_utf8(output.stdout).unwrap();
	let lines: Vec<_> = stdout.lines().collect();
	assert_eq!(lines.len(), SCREENS.len());
	for (line, screen) in lines.iter().zip(&SCREENS) {
		let (width, height) = screen.viewport;
		assert!(line.starts_with(screen.slug), "{line}");
		assert!(line.contains(screen.title), "{line}");
		assert!(line.ends_with(&format!("({width}×{height})")), "{line}");
	}
}

#[test]
fn unknown_commands_print_usage() {
	for args in [&[][..], &["open"], &["window"], &["list", "extra"]] {
		let output = gallery(args);
		assert_eq!(output.status.code(), Some(2), "{args:?}");
		let stderr = String::from_utf8(output.stderr).unwrap();
		assert!(stderr.starts_with("usage: gallery <command>"), "{stderr}");
	}
}

#[test]
fn window_rejects_an_unknown_screen_before_opening_one() {
	let output = gallery(&["window", "missing"]);
	assert_eq!(output.status.code(), Some(1));
	let stderr = String::from_utf8(output.stderr).unwrap();
	assert!(
		stderr.starts_with("gallery: no screen named `missing`; try one of: counter"),
		"{stderr}"
	);
}

/// Renders every screen through the binary with the desktop's native widgets.
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
#[test]
fn snapshot_renders_every_screen_with_native_widgets() {
	let out = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gallery-snapshot");
	if out.exists() {
		std::fs::remove_dir_all(&out).unwrap();
	}
	let output = gallery(&["snapshot", out.to_str().unwrap()]);
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(output.status.success(), "{stderr}");
	let stdout = String::from_utf8(output.stdout).unwrap();
	let printed: Vec<_> = stdout.lines().collect();
	assert_eq!(printed.len(), SCREENS.len(), "{stdout}");
	for (line, screen) in printed.iter().zip(&SCREENS) {
		let path = out.join(format!("{}.png", screen.slug));
		assert_eq!(*line, path.display().to_string());
		let png = std::fs::read(&path).unwrap();
		assert!(
			png.starts_with(b"\x89PNG\r\n\x1a\n"),
			"`{}` is not a PNG",
			screen.slug
		);
		// A blank capture compresses to a few hundred bytes; a rendered
		// screen is far larger.
		assert!(
			png.len() > 10_000,
			"`{}` rendered only {} bytes",
			screen.slug,
			png.len()
		);
	}
}

#[test]
fn gallery_ios_explains_it_needs_an_app_bundle() {
	let output = Command::new(env!("CARGO_BIN_EXE_gallery-ios"))
		.output()
		.expect("the gallery-ios binary runs");
	assert_eq!(output.status.code(), Some(1));
	let stderr = String::from_utf8(output.stderr).unwrap();
	assert!(
		stderr.starts_with("gallery-ios runs inside an iOS application bundle"),
		"{stderr}"
	);
}
