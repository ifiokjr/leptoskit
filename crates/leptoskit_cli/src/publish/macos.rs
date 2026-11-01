//! The macOS app outside the Mac App Store: signed with a Developer ID,
//! with the hardened runtime and a secure timestamp, notarized by Apple,
//! and stapled, as a `.dmg` and a `.zip` that Gatekeeper opens on any Mac.

use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;

use super::app_store::KeyFile;
use super::tools::Runner;
use super::tools::Tool;

/// The credential: the Developer ID Application identity in the keychain.
pub const IDENTITY: &str = "LEPTOSKIT_PUBLISH_MACOS_IDENTITY";
/// Where to make it.
pub const HOW: &str = "create a Developer ID Application certificate in Xcode (Settings, \
                       Accounts, Manage Certificates) or on developer.apple.com, then set this to \
                       its name, `Developer ID Application: Your Name (TEAMID)`, as `security \
                       find-identity -v -p codesigning` lists it";

/// What one distribution signs and writes.
pub struct Plan<'a> {
	/// The app `leptoskit native build macos --release` assembled.
	pub built: &'a Path,
	/// Where the `.app`, `.zip`, and `.dmg` go.
	pub out: &'a Path,
	/// The app's name, for the files and the disk image's volume.
	pub name: &'a str,
	/// `CFBundleShortVersionString`.
	pub version: &'a str,
	/// `CFBundleVersion`, when one was chosen.
	pub build: Option<u64>,
	/// The signing identity.
	pub identity: &'a str,
	/// The Safari extension's entitlements, when the app carries it.
	pub safari_entitlements: Option<&'a Path>,
}

/// What a distribution wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifacts {
	pub app: PathBuf,
	pub zip: PathBuf,
	pub dmg: PathBuf,
}

impl Plan<'_> {
	/// The files it writes.
	#[must_use]
	pub fn artifacts(&self) -> Artifacts {
		Artifacts {
			app: self.out.join(format!("{}.app", self.name)),
			zip: self.out.join(format!("{}.zip", self.name)),
			dmg: self.out.join(format!("{}.dmg", self.name)),
		}
	}
}

/// `codesign` with the identity, the hardened runtime, and a secure
/// timestamp, which notarization requires.
#[must_use]
pub fn sign(identity: &str, path: &Path, entitlements: Option<&Path>) -> Tool {
	let tool = Tool::new("codesign").args([
		"--force",
		"--sign",
		identity,
		"--options",
		"runtime",
		"--timestamp",
	]);

	match entitlements {
		Some(entitlements) => tool.arg("--entitlements").path(entitlements),
		None => tool,
	}
	.path(path)
}

/// `notarytool submit --wait`, with the App Store Connect API key, printing
/// the outcome as JSON.
#[must_use]
pub fn notarize(path: &Path, key: &KeyFile) -> Tool {
	Tool::new("xcrun")
		.args(["notarytool", "submit"])
		.path(path)
		.args(key.notarytool_args())
		.args(["--wait", "--output-format", "json"])
}

/// Checks `notarytool`'s JSON outcome.
///
/// # Errors
///
/// When Apple didn't accept the submission; the error names the command
/// that shows why.
pub fn accepted(output: &str) -> Result<()> {
	let outcome: serde_json::Value = serde_json::from_str(output.trim())
		.with_context(|| format!("notarytool answered something that isn't JSON: {output}"))?;
	let status = outcome["status"].as_str().unwrap_or_default();
	anyhow::ensure!(
		status == "Accepted",
		"Apple's notary service answered {}: run `xcrun notarytool log {}` with the same key to \
		 see why",
		if status.is_empty() { "nothing" } else { status },
		outcome["id"].as_str().unwrap_or("<submission id>")
	);
	Ok(())
}

/// Sets the copy's versions, signs it inside out, and writes the notarized
/// `.dmg` and the `.zip` of the stapled app.
///
/// # Errors
///
/// When a tool fails, or Apple's notary service doesn't accept the app.
pub fn distribute(runner: &dyn Runner, plan: &Plan<'_>, key: &KeyFile) -> Result<Artifacts> {
	let artifacts = plan.artifacts();
	std::fs::create_dir_all(plan.out)?;

	for stale in [&artifacts.app, &artifacts.zip, &artifacts.dmg] {
		if stale.is_dir() {
			std::fs::remove_dir_all(stale)?;
		} else if stale.exists() {
			std::fs::remove_file(stale)?;
		}
	}

	// `ditto` keeps the bundle's symlinks and metadata, which `codesign`
	// seals.
	runner.run(&Tool::new("ditto").path(plan.built).path(&artifacts.app))?;

	let appex = artifacts
		.app
		.join("Contents/PlugIns")
		.join(format!("{}.appex", crate::native::safari::NAME));
	let mut plists = vec![artifacts.app.join("Contents/Info.plist")];
	let extension = plan.safari_entitlements.filter(|_| appex.is_dir());

	if extension.is_some() {
		plists.push(appex.join("Contents/Info.plist"));
	}

	for plist in &plists {
		set_versions(plist, plan.version, plan.build)?;
	}

	// Inside out: the extension is sealed before the app that contains it.
	if let Some(entitlements) = extension {
		runner.run(&sign(plan.identity, &appex, Some(entitlements)))?;
	}

	runner.run(&sign(plan.identity, &artifacts.app, None))?;
	runner.run(
		&Tool::new("codesign")
			.args(["--verify", "--strict", "--deep"])
			.path(&artifacts.app),
	)?;
	runner.run(
		&Tool::new("hdiutil")
			.args(["create", "-volname", plan.name, "-srcfolder"])
			.path(&artifacts.app)
			.args(["-ov", "-format", "UDZO"])
			.path(&artifacts.dmg),
	)?;
	runner.run(&sign(plan.identity, &artifacts.dmg, None))?;
	// Notarizing the disk image covers the app inside it, so one submission
	// gives both their tickets.
	accepted(&runner.output(&notarize(&artifacts.dmg, key))?)?;

	for stapled in [&artifacts.dmg, &artifacts.app] {
		runner.run(&Tool::new("xcrun").args(["stapler", "staple"]).path(stapled))?;
	}

	runner.run(
		&Tool::new("ditto")
			.args(["-c", "-k", "--keepParent"])
			.path(&artifacts.app)
			.path(&artifacts.zip),
	)?;

	Ok(artifacts)
}

/// Sets `CFBundleShortVersionString`, and `CFBundleVersion` when a build
/// number was chosen, in the copy about to be signed.
fn set_versions(path: &Path, version: &str, build: Option<u64>) -> Result<()> {
	let mut plist = plist::Value::from_file(path)
		.with_context(|| format!("could not read {}", path.display()))?;
	let dictionary = plist
		.as_dictionary_mut()
		.with_context(|| format!("{} must contain a dictionary", path.display()))?;
	dictionary.insert("CFBundleShortVersionString".into(), version.into());

	if let Some(build) = build {
		dictionary.insert("CFBundleVersion".into(), build.to_string().into());
	}

	plist
		.to_file_xml(path)
		.with_context(|| format!("could not write {}", path.display()))
}

/// The GitHub release `tag` in `repository`, made if it's missing, with the
/// files attached; files of the same name are replaced.
///
/// # Errors
///
/// When `gh` fails.
pub fn release(
	runner: &dyn Runner,
	repository: &str,
	tag: &str,
	title: &str,
	notes: &str,
	files: &[&Path],
) -> Result<()> {
	let exists = runner
		.output(
			&Tool::new("gh")
				.args(["release", "view", tag, "--repo", repository])
				.args(["--json", "tagName"]),
		)
		.is_ok();
	let tool = if exists {
		Tool::new("gh").args(["release", "upload", tag, "--repo", repository, "--clobber"])
	} else {
		Tool::new("gh").args([
			"release", "create", tag, "--repo", repository, "--title", title, "--notes", notes,
		])
	};

	runner.run(&files.iter().fold(tool, |tool, file| tool.path(file)))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::publish::secrets::Secret;
	use crate::publish::tools::Recorder;

	fn key() -> KeyFile {
		KeyFile::at(
			PathBuf::from("/keys/AuthKey_KEY123.p8"),
			Secret::new("KEY123".into()),
			Secret::new("issuer-1".into()),
		)
	}

	/// A built app, as `native build macos` assembles it, with its Safari
	/// extension.
	fn built(root: &Path) -> PathBuf {
		let app = root.join("App.app");
		let info = include_str!("../native/templates/MacInfo.plist")
			.replace("__PACKAGE__", "com.example.notes")
			.replace("__NAME__", "notes")
			.replace("__VERSION__", "0.0.1");
		std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
		std::fs::write(app.join("Contents/Info.plist"), &info).unwrap();
		let appex = app.join("Contents/PlugIns/SafariExtension.appex/Contents");
		std::fs::create_dir_all(&appex).unwrap();
		std::fs::write(appex.join("Info.plist"), &info).unwrap();
		app
	}

	#[test]
	fn the_app_is_signed_inside_out_notarized_and_stapled() {
		let base = std::fs::canonicalize(std::env::temp_dir()).unwrap();
		let root = tempfile::tempdir_in(base).unwrap();
		let app = built(root.path());
		let out = root.path().join("out");
		let entitlements = root.path().join("entitlements.plist");
		let recorder = Recorder {
			seen: std::cell::RefCell::default(),
			answer: Box::new(|tool| {
				if tool.args.first().is_some_and(|arg| arg == "notarytool") {
					r#"{"id":"n-1","status":"Accepted"}"#.to_owned()
				} else {
					String::new()
				}
			}),
		};
		let plan = Plan {
			built: &app,
			out: &out,
			name: "Notes",
			version: "1.4.0",
			build: Some(7),
			identity: "Developer ID Application: Example (ABCDE12345)",
			safari_entitlements: Some(&entitlements),
		};
		let runner = CopyingRecorder {
			inner: recorder,
			from: app.clone(),
		};

		let artifacts = distribute(&runner, &plan, &key()).unwrap();
		assert_eq!(artifacts.dmg, out.join("Notes.dmg"));

		let relative = |text: &str| text.replace(&root.path().display().to_string(), "<root>");
		let seen: Vec<String> = runner
			.inner
			.seen
			.borrow()
			.iter()
			.map(|tool| relative(&format!("{} {}", tool.program, tool.args.join(" "))))
			.collect();
		insta::assert_snapshot!(seen.join("\n"), @r"
		ditto <root>/App.app <root>/out/Notes.app
		codesign --force --sign Developer ID Application: Example (ABCDE12345) --options runtime --timestamp --entitlements <root>/entitlements.plist <root>/out/Notes.app/Contents/PlugIns/SafariExtension.appex
		codesign --force --sign Developer ID Application: Example (ABCDE12345) --options runtime --timestamp <root>/out/Notes.app
		codesign --verify --strict --deep <root>/out/Notes.app
		hdiutil create -volname Notes -srcfolder <root>/out/Notes.app -ov -format UDZO <root>/out/Notes.dmg
		codesign --force --sign Developer ID Application: Example (ABCDE12345) --options runtime --timestamp <root>/out/Notes.dmg
		xcrun notarytool submit <root>/out/Notes.dmg --key /keys/AuthKey_KEY123.p8 --key-id KEY123 --issuer issuer-1 --wait --output-format json
		xcrun stapler staple <root>/out/Notes.dmg
		xcrun stapler staple <root>/out/Notes.app
		ditto -c -k --keepParent <root>/out/Notes.app <root>/out/Notes.zip
		");

		// The copy carries the version; the build is untouched.
		for plist in [
			"Contents/Info.plist",
			"Contents/PlugIns/SafariExtension.appex/Contents/Info.plist",
		] {
			let plist = plist::Value::from_file(artifacts.app.join(plist)).unwrap();
			let plist = plist.as_dictionary().unwrap();
			let get = |key: &str| plist.get(key).and_then(plist::Value::as_string).unwrap();
			assert_eq!(get("CFBundleShortVersionString"), "1.4.0");
			assert_eq!(get("CFBundleVersion"), "7");
		}
		let original = plist::Value::from_file(app.join("Contents/Info.plist")).unwrap();
		assert_eq!(
			original
				.as_dictionary()
				.unwrap()
				.get("CFBundleShortVersionString")
				.and_then(plist::Value::as_string),
			Some("0.0.1")
		);
	}

	/// Records tools, and copies the bundle when `ditto` would.
	struct CopyingRecorder {
		inner: Recorder,
		from: PathBuf,
	}

	impl Runner for CopyingRecorder {
		fn run(&self, tool: &Tool) -> Result<()> {
			if tool.program == "ditto" && tool.args.len() == 2 {
				crate::native::safari::copy_tree(&self.from, Path::new(&tool.args[1]))?;
			}

			self.inner.run(tool)
		}

		fn output(&self, tool: &Tool) -> Result<String> {
			self.inner.output(tool)
		}
	}

	#[test]
	fn a_refused_notarization_says_how_to_see_why() {
		accepted(r#"{"id":"n-1","status":"Accepted"}"#).unwrap();
		let error = accepted(r#"{"id":"n-2","status":"Invalid"}"#)
			.unwrap_err()
			.to_string();
		assert!(
			error.contains("answered Invalid") && error.contains("xcrun notarytool log n-2"),
			"{error}"
		);
	}

	#[test]
	fn github_releases_are_made_once_and_then_updated() {
		let files = [Path::new("/out/Notes.dmg"), Path::new("/out/Notes.zip")];
		// `gh release view` succeeds: the release exists.
		let recorder = Recorder::default();
		release(
			&recorder,
			"example/notes",
			"v1.4.0",
			"Notes 1.4.0",
			"Notes.",
			&files,
		)
		.unwrap();
		assert_eq!(
			recorder.seen.borrow()[1].args.join(" "),
			"release upload v1.4.0 --repo example/notes --clobber /out/Notes.dmg /out/Notes.zip"
		);

		struct Missing(Recorder);
		impl Runner for Missing {
			fn run(&self, tool: &Tool) -> Result<()> {
				self.0.run(tool)
			}

			fn output(&self, tool: &Tool) -> Result<String> {
				self.0.output(tool)?;
				anyhow::bail!("release not found")
			}
		}
		let missing = Missing(Recorder::default());
		release(
			&missing,
			"example/notes",
			"v1.4.0",
			"Notes 1.4.0",
			"Notes.",
			&files,
		)
		.unwrap();
		assert_eq!(
			missing.0.seen.borrow()[1].args.join(" "),
			"release create v1.4.0 --repo example/notes --title Notes 1.4.0 --notes Notes. \
			 /out/Notes.dmg /out/Notes.zip"
		);
	}
}
