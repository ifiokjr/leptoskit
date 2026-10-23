//! Artifact size reports for native builds.
//!
//! The web build reports what the browser downloads; the native build reports
//! what the application carries. Every platform build ends by walking its
//! artifact — the `.app` bundle or the APK — so a size change shows up in the
//! build that caused it, next to the bundle report `leptoskit build --release`
//! already prints for the web client.

use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;

use crate::report::kib;

/// The measured artifact: its files, largest first, and the total.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
	/// What was measured, such as `Release-iphonesimulator/App.app`.
	pub label: String,
	/// Every file in the artifact, sorted largest first.
	pub entries: Vec<(String, u64)>,
}

impl Report {
	/// The total size of every file in the artifact.
	#[must_use]
	pub fn total(&self) -> u64 {
		self.entries.iter().map(|(_, bytes)| bytes).sum()
	}

	/// The report as an indented table of every file and the total.
	#[must_use]
	pub fn render(&self) -> String {
		use std::fmt::Write as _;
		let width = self
			.entries
			.iter()
			.map(|(path, _)| path.len())
			.chain([self.label.len()])
			.max()
			.unwrap_or_default()
			.max(6);
		let mut out = String::new();
		for (path, bytes) in &self.entries {
			let _ = writeln!(out, "  {path:width$}  {:>10}", kib(*bytes));
		}
		let _ = write!(out, "  {:width$}  {:>10}", "total", kib(self.total()));
		out
	}

	/// Measures every file under `directory`, named relative to it.
	pub fn directory(label: &str, directory: &Path) -> Result<Self> {
		let mut entries = Vec::new();
		for entry in walkdir::WalkDir::new(directory) {
			let entry = entry.with_context(|| format!("walking {}", directory.display()))?;
			if !entry.file_type().is_file() {
				continue;
			}
			let relative = entry
				.path()
				.strip_prefix(directory)
				.unwrap_or(entry.path())
				.to_string_lossy()
				.replace('\\', "/");
			let bytes = entry
				.metadata()
				.with_context(|| format!("reading {}", entry.path().display()))?
				.len();
			entries.push((relative, bytes));
		}
		Ok(Self::sorted(label, entries))
	}

	/// Measures one file.
	pub fn file(label: &str, path: &Path) -> Result<Self> {
		let bytes = std::fs::metadata(path)
			.with_context(|| format!("reading {}", path.display()))?
			.len();
		let name = path.file_name().map_or_else(
			|| path.display().to_string(),
			|name| name.to_string_lossy().into_owned(),
		);
		Ok(Self::sorted(label, vec![(name, bytes)]))
	}

	fn sorted(label: &str, mut entries: Vec<(String, u64)>) -> Self {
		entries.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
		Self {
			label: label.to_owned(),
			entries,
		}
	}
}

/// Reports the macOS application bundle at `.leptoskit/macos/App.app`.
pub fn macos(root: &Path) -> Result<Report> {
	Report::directory("macos/App.app", &root.join(".leptoskit/macos/App.app"))
}

/// Reports the newest application bundle xcodebuild produced under
/// `.leptoskit/ios/build/Build/Products`.
pub fn ios(root: &Path, release: bool) -> Result<Option<Report>> {
	let products = root.join(".leptoskit/ios/build/Build/Products");
	let configuration = if release { "Release" } else { "Debug" };
	let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
	let directories = match std::fs::read_dir(&products) {
		Ok(entries) => entries,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
		Err(error) => return Err(error).context("reading the xcodebuild products"),
	};
	for entry in directories {
		let entry = entry?;
		let path = entry.path().join("App.app");
		if !entry
			.file_name()
			.to_string_lossy()
			.starts_with(configuration)
			|| !path.join("App").is_file()
		{
			continue;
		}
		let modified = entry
			.metadata()?
			.modified()
			.with_context(|| format!("reading {}", entry.path().display()))?;
		if newest.as_ref().is_none_or(|(time, _)| modified > *time) {
			newest = Some((modified, path));
		}
	}
	let Some((_, app)) = newest else {
		return Ok(None);
	};
	// `Path::file_name` rather than splitting the string: the generated
	// project's absolute path uses the platform separator.
	let label = app
		.parent()
		.and_then(Path::file_name)
		.map_or_else(|| "ios".into(), |name| name.to_string_lossy().into_owned());
	Ok(Some(Report::directory(&format!("{label}/App.app"), &app)?))
}

/// Reports the APK Gradle produced, plus the assets it carries.
pub fn android(root: &Path, release: bool) -> Result<Option<Report>> {
	let variant = if release { "release" } else { "debug" };
	let outputs = root
		.join("hosts/android/app/build/outputs/apk")
		.join(variant);
	let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
	let directories = match std::fs::read_dir(&outputs) {
		Ok(entries) => entries,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
		Err(error) => return Err(error).context("reading the Gradle APK outputs"),
	};
	for entry in directories {
		let entry = entry?;
		if entry
			.path()
			.extension()
			.is_none_or(|extension| extension != "apk")
		{
			continue;
		}
		let modified = entry
			.metadata()?
			.modified()
			.with_context(|| format!("reading {}", entry.path().display()))?;
		if newest.as_ref().is_none_or(|(time, _)| modified > *time) {
			newest = Some((modified, entry.path()));
		}
	}
	let Some((_, apk)) = newest else {
		return Ok(None);
	};
	let mut report = Report::file(variant, &apk)?;
	// The carried assets the APK bundles, from the directory Gradle packs.
	for assets in [
		root.join(".leptoskit/mobile"),
		root.join(".leptoskit/assets"),
	] {
		for entry in walkdir::WalkDir::new(&assets) {
			let entry = entry.ok();
			let Some(path) = entry.as_ref().map(walkdir::DirEntry::path) else {
				continue;
			};
			if !path.is_file() {
				continue;
			}
			let name = path
				.strip_prefix(&assets)
				.unwrap_or(path)
				.to_string_lossy()
				.into_owned();
			let bytes = std::fs::metadata(path).map_or(0, |metadata| metadata.len());
			report.entries.push((format!("assets/{name}"), bytes));
		}
	}
	report
		.entries
		.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
	Ok(Some(report))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn scratch(tag: &str) -> PathBuf {
		let root = std::env::temp_dir()
			.canonicalize()
			.unwrap()
			.join(format!("leptoskit-size-{tag}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		root
	}

	#[test]
	fn directory_reports_are_sorted_largest_first_with_a_total() {
		let root = scratch("report");
		std::fs::create_dir_all(root.join("inner")).unwrap();
		std::fs::write(root.join("large"), vec![0_u8; 2048]).unwrap();
		std::fs::write(root.join("inner/small"), vec![0_u8; 512]).unwrap();
		let report = Report::directory("app", &root).unwrap();
		assert_eq!(
			report.entries,
			vec![("large".to_owned(), 2048), ("inner/small".to_owned(), 512),]
		);
		assert_eq!(report.total(), 2560);
		let rendered = report.render();
		assert!(rendered.contains("large"));
		assert!(rendered.contains("total"));
		assert!(rendered.contains("2.5 KiB"));
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn file_reports_the_artifact_by_name() {
		let root = scratch("file");
		std::fs::write(root.join("app-release-unsigned.apk"), vec![0_u8; 1024]).unwrap();
		let report = Report::file("release", &root.join("app-release-unsigned.apk")).unwrap();
		assert_eq!(
			report.entries,
			vec![("app-release-unsigned.apk".to_owned(), 1024)]
		);
		assert_eq!(report.total(), 1024);
		// A path with no filename reports the whole path as its name.
		let root_name = Report::file("root", Path::new("/")).unwrap();
		assert_eq!(root_name.entries.len(), 1);
		// A vanished artifact is an error, not an empty report.
		assert!(
			Report::file("release", &root.join("missing.apk"))
				.err()
				.is_some_and(|error| error.to_string().contains("reading"))
		);
		assert!(
			Report::directory("app", &root.join("missing"))
				.err()
				.is_some_and(|error| error.to_string().contains("walking"))
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn macos_measures_the_application_bundle() {
		let root = scratch("macos");
		let contents = root.join(".leptoskit/macos/App.app/Contents");
		std::fs::create_dir_all(contents.join("MacOS")).unwrap();
		std::fs::create_dir_all(contents.join("Resources")).unwrap();
		std::fs::write(contents.join("MacOS/App"), vec![0_u8; 4096]).unwrap();
		std::fs::write(contents.join("Resources/app.wasm"), vec![0_u8; 256]).unwrap();
		std::fs::write(contents.join("Info.plist"), b"plist").unwrap();
		let report = macos(&root).unwrap();
		assert_eq!(report.total(), 4357);
		assert!(
			report
				.entries
				.contains(&("Contents/MacOS/App".to_owned(), 4096))
		);
		assert!(
			report
				.entries
				.contains(&("Contents/Resources/app.wasm".to_owned(), 256))
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn ios_measures_the_newest_release_application_and_skips_without_one() {
		let root = scratch("ios");
		let products = root.join(".leptoskit/ios/build/Build/Products");
		for (configuration, binary) in [
			("Debug-iphonesimulator", 128),
			("Release-iphonesimulator", 512),
		] {
			let app = products.join(configuration).join("App.app");
			std::fs::create_dir_all(&app).unwrap();
			std::fs::write(app.join("App"), vec![0_u8; binary]).unwrap();
			std::fs::write(app.join("app.lkb"), vec![0_u8; 64]).unwrap();
		}
		let report = ios(&root, true).unwrap().expect("a release app");
		assert_eq!(report.label, "Release-iphonesimulator/App.app");
		assert_eq!(report.total(), 576);
		// A debug build asks for the debug products.
		let debug = ios(&root, false).unwrap().expect("a debug app");
		assert_eq!(debug.label, "Debug-iphonesimulator/App.app");
		std::fs::remove_dir_all(root).unwrap();

		let empty = scratch("ios-empty");
		assert!(ios(&empty, true).unwrap().is_none());
		// A release build finds nothing when only debug products exist.
		std::fs::create_dir_all(
			empty.join(".leptoskit/ios/build/Build/Products/Debug-iphonesimulator"),
		)
		.unwrap();
		assert!(ios(&empty, true).unwrap().is_none());
		// Something other than a directory at the products path is an error.
		let blocked = scratch("ios-blocked");
		std::fs::create_dir_all(blocked.join(".leptoskit/ios/build/Build")).unwrap();
		std::fs::write(blocked.join(".leptoskit/ios/build/Build/Products"), b"file").unwrap();
		assert!(ios(&blocked, true).is_err());
		std::fs::remove_dir_all(empty).unwrap();
		std::fs::remove_dir_all(blocked).unwrap();
	}

	#[test]
	fn android_measures_the_newest_apk_and_the_carried_assets() {
		let root = scratch("android");
		let outputs = root.join("hosts/android/app/build/outputs/apk/release");
		std::fs::create_dir_all(&outputs).unwrap();
		let apk = outputs.join("app-release-unsigned.apk");
		std::fs::write(&apk, vec![0_u8; 2048]).unwrap();
		// An older APK and a non-APK file must not win.
		let older = outputs.join("app-release-older.apk");
		std::fs::write(&older, vec![0_u8; 16]).unwrap();
		std::fs::write(outputs.join("output-metadata.json"), b"{}").unwrap();
		let now = std::time::SystemTime::now();
		set_modified(&apk, now);
		set_modified(&older, now - std::time::Duration::from_hours(1));
		let mobile = root.join(".leptoskit/mobile");
		std::fs::create_dir_all(&mobile).unwrap();
		std::fs::write(mobile.join("app.lkb"), vec![0_u8; 128]).unwrap();
		std::fs::write(mobile.join("codepush.json"), b"{}").unwrap();
		let report = android(&root, true).unwrap().expect("an APK");
		assert!(
			report
				.entries
				.contains(&("app-release-unsigned.apk".to_owned(), 2048)),
			"the newest APK leads the report: {:?}",
			report.entries
		);
		assert!(report.entries.contains(&("assets/app.lkb".to_owned(), 128)));
		assert!(
			!report
				.entries
				.iter()
				.any(|(path, _)| path.contains("older"))
		);

		let empty = scratch("android-empty");
		assert!(android(&empty, true).unwrap().is_none());
		// Outputs without an APK report nothing; something other than a
		// directory at the outputs path is an error.
		let outputs = empty.join("hosts/android/app/build/outputs/apk/release");
		std::fs::create_dir_all(&outputs).unwrap();
		assert!(android(&empty, true).unwrap().is_none());
		let blocked = scratch("android-blocked");
		std::fs::create_dir_all(blocked.join("hosts/android/app/build/outputs/apk")).unwrap();
		std::fs::write(
			blocked.join("hosts/android/app/build/outputs/apk/release"),
			b"file",
		)
		.unwrap();
		assert!(android(&blocked, true).is_err());
		std::fs::remove_dir_all(root).unwrap();
		std::fs::remove_dir_all(empty).unwrap();
		std::fs::remove_dir_all(blocked).unwrap();
	}

	/// Sets a file's modification time, so newest-wins selection is
	/// deterministic in tests.
	fn set_modified(path: &Path, time: std::time::SystemTime) {
		let file = std::fs::File::options().write(true).open(path).unwrap();
		file.set_times(
			std::fs::FileTimes::new()
				.set_modified(time)
				.set_accessed(time),
		)
		.unwrap();
	}
}
