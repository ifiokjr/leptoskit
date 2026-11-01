//! App Store Connect: the iOS app, with the Safari extension inside it.
//!
//! Xcode archives the generated project and uploads it itself, signing
//! automatically with the API key, so no certificate or provisioning profile
//! is installed by hand. The API then waits for the build to process, adds
//! it to TestFlight groups, and, with `--submit`, sends a version to review.

use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use serde_json::Value;
use serde_json::json;

use super::http::Body;
use super::http::Http;
use super::http::Reply;
use super::http::poll;
use super::secrets::Secret;
use super::secrets::es256;
use super::secrets::issued_at;
use super::tools::Tool;

/// The API's address.
pub const API: &str = "https://api.appstoreconnect.apple.com";
/// The credentials: an App Store Connect API key.
pub const ISSUER: &str = "LEPTOSKIT_PUBLISH_APPLE_ISSUER_ID";
pub const KEY_ID: &str = "LEPTOSKIT_PUBLISH_APPLE_KEY_ID";
pub const KEY: &str = "LEPTOSKIT_PUBLISH_APPLE_KEY";
/// Where to make them.
pub const HOW: &str = "in App Store Connect, open Users and Access, Integrations, App Store \
                       Connect API, and generate a team key with the App Manager role: the page \
                       shows the issuer ID and the key ID, and LEPTOSKIT_PUBLISH_APPLE_KEY is the \
                       downloaded .p8 file's path or its contents";

/// An App Store Connect API key.
pub struct Key {
	issuer: Secret,
	id: Secret,
	/// The `.p8` file's text.
	pem: Secret,
	/// The file it came from, when it came from one.
	path: Option<PathBuf>,
}

impl Key {
	/// The key from its three credentials. `key` is the `.p8` file's path or
	/// its contents.
	///
	/// # Errors
	///
	/// When `key` names a file that can't be read.
	pub fn new(issuer: Secret, id: Secret, key: &Secret) -> Result<Self> {
		let text = key.expose().trim();
		let (pem, path) = if text.contains("-----BEGIN") {
			(key.clone(), None)
		} else {
			let path = PathBuf::from(text);
			let pem = std::fs::read_to_string(&path)
				.with_context(|| format!("{KEY} names a file that can't be read"))?;
			(Secret::new(pem), Some(path))
		};
		Ok(Self {
			issuer,
			id,
			pem,
			path,
		})
	}

	/// A token for the API: issued now, for 20 minutes, the longest Apple
	/// accepts.
	///
	/// # Errors
	///
	/// When the key isn't a P-256 key.
	pub fn token(&self) -> Result<Secret> {
		let issued = issued_at();
		es256(
			&json!({
				"iss": self.issuer.expose(),
				"iat": issued,
				"exp": issued + 20 * 60,
				"aud": "appstoreconnect-v1",
			}),
			self.id.expose(),
			self.pem.expose(),
		)
		.map(Secret::new)
	}

	/// The key as a file, for `xcodebuild` and `notarytool`, which read it
	/// from disk: the file it came from, or a private copy in `dir` that is
	/// removed with the returned guard.
	///
	/// # Errors
	///
	/// When the copy can't be written.
	pub fn file(&self, dir: &Path) -> Result<KeyFile> {
		if let Some(path) = &self.path {
			return Ok(KeyFile {
				path: path.clone(),
				_temporary: None,
				id: self.id.clone(),
				issuer: self.issuer.clone(),
			});
		}
		std::fs::create_dir_all(dir)?;
		let temporary = tempfile::Builder::new()
			.prefix("leptoskit-key-")
			.tempdir_in(dir)?;
		let path = temporary
			.path()
			.join(format!("AuthKey_{}.p8", self.id.expose()));
		write_private(&path, self.pem.expose())?;
		Ok(KeyFile {
			path,
			_temporary: Some(temporary),
			id: self.id.clone(),
			issuer: self.issuer.clone(),
		})
	}
}

/// Writes `text` to `path`, readable only by this user.
fn write_private(path: &Path, text: &str) -> Result<()> {
	use std::io::Write as _;

	let mut options = std::fs::OpenOptions::new();
	options.write(true).create_new(true);
	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt as _;
		options.mode(0o600);
	}
	options
		.open(path)
		.and_then(|mut file| file.write_all(text.as_bytes()))
		.context("could not write the App Store Connect key for Xcode")
}

/// The API key on disk, for the tools that read it from there.
pub struct KeyFile {
	pub path: PathBuf,
	_temporary: Option<tempfile::TempDir>,
	id: Secret,
	issuer: Secret,
}

impl KeyFile {
	/// A key file at `path`, for tests of the tools that read it.
	#[cfg(test)]
	pub fn at(path: PathBuf, id: Secret, issuer: Secret) -> Self {
		Self {
			path,
			_temporary: None,
			id,
			issuer,
		}
	}

	/// `xcodebuild`'s arguments for signing in with the key.
	#[must_use]
	pub fn xcodebuild_args(&self) -> Vec<String> {
		vec![
			"-allowProvisioningUpdates".to_owned(),
			"-authenticationKeyPath".to_owned(),
			self.path.display().to_string(),
			"-authenticationKeyID".to_owned(),
			self.id.expose().to_owned(),
			"-authenticationKeyIssuerID".to_owned(),
			self.issuer.expose().to_owned(),
		]
	}

	/// `notarytool`'s arguments for signing in with the key.
	#[must_use]
	pub fn notarytool_args(&self) -> Vec<String> {
		vec![
			"--key".to_owned(),
			self.path.display().to_string(),
			"--key-id".to_owned(),
			self.id.expose().to_owned(),
			"--issuer".to_owned(),
			self.issuer.expose().to_owned(),
		]
	}
}

/// The generated Xcode project, relative to the app's root.
pub const PROJECT: &str = ".leptoskit/ios/Host.xcodeproj";

/// `xcodebuild archive` for the generated project: signed automatically for
/// `team`, as `version` (`CFBundleShortVersionString`) build `build`
/// (`CFBundleVersion`), into `out/App.xcarchive`.
#[must_use]
pub fn archive(
	root: &Path,
	out: &Path,
	team: &str,
	version: &str,
	build: u64,
	key: &KeyFile,
) -> Tool {
	Tool::new("xcodebuild")
		.dir(root)
		// Xcode reads these as its linker drivers; a nix or devenv shell sets
		// `LD=ld`, which rejects the flags Xcode passes it.
		.unset("LD")
		.unset("LDPLUSPLUS")
		.args(["archive", "-project", PROJECT, "-scheme", "App"])
		.args(["-configuration", "Release"])
		.args(["-destination", "generic/platform=iOS"])
		.arg("-archivePath")
		.path(&out.join("App.xcarchive"))
		.args(["-derivedDataPath", ".leptoskit/ios/build"])
		.args(key.xcodebuild_args())
		.arg(format!("DEVELOPMENT_TEAM={team}"))
		.arg("CODE_SIGN_STYLE=Automatic")
		.arg(format!("MARKETING_VERSION={version}"))
		.arg(format!("CURRENT_PROJECT_VERSION={build}"))
}

/// `xcodebuild -exportArchive`, which signs the archive for the App Store
/// and, with [`export_options`], uploads it to App Store Connect.
#[must_use]
pub fn export(out: &Path, key: &KeyFile) -> Tool {
	Tool::new("xcodebuild")
		.unset("LD")
		.unset("LDPLUSPLUS")
		.arg("-exportArchive")
		.arg("-archivePath")
		.path(&out.join("App.xcarchive"))
		.arg("-exportPath")
		.path(&out.join("export"))
		.arg("-exportOptionsPlist")
		.path(&out.join("ExportOptions.plist"))
		.args(key.xcodebuild_args())
}

/// The `ExportOptions.plist` that makes Xcode upload the build itself, and
/// keep the build number leptoskit chose.
#[must_use]
pub fn export_options(team: &str) -> String {
	format!(
		r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>destination</key>
	<string>upload</string>
	<key>manageAppVersionAndBuildNumber</key>
	<false/>
	<key>method</key>
	<string>app-store-connect</string>
	<key>signingStyle</key>
	<string>automatic</string>
	<key>teamID</key>
	<string>{team}</string>
	<key>uploadSymbols</key>
	<true/>
</dict>
</plist>
"#
	)
}

/// A query parameter's name, with its brackets encoded: `filter[app]`.
fn filter(name: &str) -> String {
	name.replace('[', "%5B").replace(']', "%5D")
}

/// App Store versions that can still be edited.
const EDITABLE: [&str; 6] = [
	"PREPARE_FOR_SUBMISSION",
	"DEVELOPER_REJECTED",
	"REJECTED",
	"METADATA_REJECTED",
	"INVALID_BINARY",
	"READY_FOR_REVIEW",
];

/// A TestFlight group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
	pub id: String,
	pub name: String,
	/// Internal groups are the team's own testers; external ones need
	/// Apple's beta review.
	pub internal: bool,
	/// An internal group that gets every build without being added.
	pub every_build: bool,
}

/// A client for App Store Connect.
pub struct AppStore<'a> {
	http: &'a Http,
	api: String,
	key: &'a Key,
}

impl<'a> AppStore<'a> {
	#[must_use]
	pub fn new(http: &'a Http, api: &str, key: &'a Key) -> Self {
		Self {
			http,
			api: api.trim_end_matches('/').to_owned(),
			key,
		}
	}

	fn send(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Reply> {
		let authorization = format!("Bearer {}", self.key.token()?.expose());
		let reply = self.http.send(
			method,
			&format!("{}{path}", self.api),
			&[("authorization", &authorization)],
			body.map(Body::json),
		)?;
		if reply.status == 401 {
			anyhow::bail!(
				"App Store Connect refused the API key: check {KEY_ID} and {ISSUER}, and that the \
				 key wasn't revoked under Users and Access, Integrations"
			);
		}
		Ok(reply)
	}

	fn json(&self, method: &str, path: &str, body: Option<&Value>, action: &str) -> Result<Value> {
		self.send(method, path, body)?.ok(action)?.json()
	}

	/// The highest build number uploaded for the app, if there is one.
	///
	/// # Errors
	///
	/// When App Store Connect refuses.
	pub fn highest_build(&self, app: &str) -> Result<Option<u64>> {
		let builds = self.json(
			"GET",
			&format!(
				"/v1/builds?{}={app}&sort=-uploadedDate&limit=200&{}=version",
				filter("filter[app]"),
				filter("fields[builds]")
			),
			None,
			"reading the app's builds",
		)?;
		Ok(builds["data"]
			.as_array()
			.into_iter()
			.flatten()
			.filter_map(|build| build["attributes"]["version"].as_str())
			// A dotted build number compares by its first part.
			.filter_map(|version| version.split('.').next()?.parse::<u64>().ok())
			.max())
	}

	/// Waits until App Store Connect has processed the uploaded build, and
	/// returns its ID.
	///
	/// # Errors
	///
	/// When processing fails, or doesn't finish within an hour.
	pub fn wait_for_build(&self, app: &str, version: &str, build: u64) -> Result<String> {
		let path = format!(
			"/v1/builds?{}={app}&{}={build}&{}={version}&limit=1",
			filter("filter[app]"),
			filter("filter[version]"),
			filter("filter[preReleaseVersion.version]")
		);
		poll(
			"App Store Connect's processing of the build",
			120,
			Duration::from_secs(30),
			|| {
				let builds = self.json("GET", &path, None, "reading the build")?;
				let Some(found) = builds["data"].get(0) else {
					// An upload takes a few minutes to appear.
					return Ok(None);
				};
				match found["attributes"]["processingState"].as_str() {
					Some("VALID") => {
						Ok(Some(
							found["id"]
								.as_str()
								.context("App Store Connect listed a build without an ID")?
								.to_owned(),
						))
					}
					Some("FAILED" | "INVALID") => {
						anyhow::bail!(
							"App Store Connect couldn't process build {build}; its email to the \
							 team says why"
						)
					}
					_ => Ok(None),
				}
			},
		)
	}

	/// The app's TestFlight groups.
	///
	/// # Errors
	///
	/// When App Store Connect refuses.
	pub fn groups(&self, app: &str) -> Result<Vec<Group>> {
		let groups = self.json(
			"GET",
			&format!("/v1/apps/{app}/betaGroups?limit=200"),
			None,
			"reading the TestFlight groups",
		)?;
		Ok(groups["data"]
			.as_array()
			.into_iter()
			.flatten()
			.filter_map(|group| {
				let attributes = &group["attributes"];
				Some(Group {
					id: group["id"].as_str()?.to_owned(),
					name: attributes["name"].as_str()?.to_owned(),
					internal: attributes["isInternalGroup"].as_bool().unwrap_or(false),
					every_build: attributes["hasAccessToAllBuilds"]
						.as_bool()
						.unwrap_or(false),
				})
			})
			.collect())
	}

	/// Adds the build to the TestFlight groups `names`, and returns them.
	/// An internal group that gets every build already has it.
	///
	/// # Errors
	///
	/// When a group doesn't exist, or App Store Connect refuses.
	pub fn add_to_groups(&self, app: &str, build: &str, names: &[String]) -> Result<Vec<Group>> {
		if names.is_empty() {
			return Ok(Vec::new());
		}
		let groups = self.groups(app)?;
		let mut chosen = Vec::new();
		for name in names {
			let group = groups
				.iter()
				.find(|group| &group.name == name)
				.with_context(|| {
					format!(
						"the app has no TestFlight group `{name}`; it has {}",
						groups
							.iter()
							.map(|group| format!("`{}`", group.name))
							.collect::<Vec<_>>()
							.join(", ")
					)
				})?;
			chosen.push(group.clone());
		}
		let data: Vec<Value> = chosen
			.iter()
			.filter(|group| !(group.internal && group.every_build))
			.map(|group| json!({ "type": "betaGroups", "id": group.id }))
			.collect();
		if !data.is_empty() {
			self.send(
				"POST",
				&format!("/v1/builds/{build}/relationships/betaGroups"),
				Some(&json!({ "data": data })),
			)?
			.ok("adding the build to TestFlight groups")?;
		}
		Ok(chosen)
	}

	/// Sends the build to Apple's beta review, which external TestFlight
	/// groups need.
	///
	/// # Errors
	///
	/// When App Store Connect refuses, such as when the build's export
	/// compliance isn't answered yet.
	pub fn submit_for_beta_review(&self, build: &str) -> Result<()> {
		self.send(
			"POST",
			"/v1/betaAppReviewSubmissions",
			Some(&json!({ "data": {
				"type": "betaAppReviewSubmissions",
				"relationships": { "build": { "data": { "type": "builds", "id": build } } },
			}})),
		)?
		.ok("sending the build to TestFlight's beta review")
		.map(drop)
	}

	/// The app's iOS App Store version `version`, made editable: the one
	/// with that number, or the editable one renamed, or a new one. Returns
	/// its ID, and whether it is the app's first version.
	///
	/// # Errors
	///
	/// When `version` is already in review or released.
	pub fn version(&self, app: &str, version: &str) -> Result<(String, bool)> {
		let versions = self.json(
			"GET",
			&format!(
				"/v1/apps/{app}/appStoreVersions?{}=IOS&limit=200",
				filter("filter[platform]")
			),
			None,
			"reading the app's App Store versions",
		)?;
		let versions: Vec<(String, String, String)> = versions["data"]
			.as_array()
			.into_iter()
			.flatten()
			.filter_map(|found| {
				let attributes = &found["attributes"];
				Some((
					found["id"].as_str()?.to_owned(),
					attributes["versionString"].as_str()?.to_owned(),
					attributes["appStoreState"]
						.as_str()
						.or_else(|| attributes["appVersionState"].as_str())
						.unwrap_or_default()
						.to_owned(),
				))
			})
			.collect();
		// Only an app's first version has no "What's New".
		let first = |chosen: &str| versions.iter().all(|(id, ..)| id == chosen);
		if let Some((id, _, state)) = versions.iter().find(|(_, number, _)| number == version) {
			anyhow::ensure!(
				EDITABLE.contains(&state.as_str()),
				"App Store version {version} is already {}; raise app.version for a new one",
				state.to_lowercase().replace('_', " ")
			);
			return Ok((id.clone(), first(id)));
		}
		// Only one version can be in preparation, so an unsubmitted one takes
		// the new number.
		if let Some((id, ..)) = versions
			.iter()
			.find(|(_, _, state)| EDITABLE.contains(&state.as_str()))
		{
			self.send(
				"PATCH",
				&format!("/v1/appStoreVersions/{id}"),
				Some(&json!({ "data": {
					"type": "appStoreVersions", "id": id,
					"attributes": { "versionString": version },
				}})),
			)?
			.ok("renaming the App Store version in preparation")?;
			return Ok((id.clone(), first(id)));
		}
		let created = self.json(
			"POST",
			"/v1/appStoreVersions",
			Some(&json!({ "data": {
				"type": "appStoreVersions",
				"attributes": { "platform": "IOS", "versionString": version },
				"relationships": { "app": { "data": { "type": "apps", "id": app } } },
			}})),
			"creating the App Store version",
		)?;
		Ok((
			created["data"]["id"]
				.as_str()
				.context("App Store Connect created a version without an ID")?
				.to_owned(),
			versions.is_empty(),
		))
	}

	/// Sets text on a version's `locale` localization, creating it when
	/// it's missing: `attributes` such as `whatsNew`, `description`, and
	/// `keywords`.
	///
	/// # Errors
	///
	/// When App Store Connect refuses the text.
	pub fn localize(&self, version: &str, locale: &str, attributes: &Value) -> Result<()> {
		let localizations = self.json(
			"GET",
			&format!("/v1/appStoreVersions/{version}/appStoreVersionLocalizations?limit=200"),
			None,
			"reading the version's localizations",
		)?;
		let existing = localizations["data"]
			.as_array()
			.into_iter()
			.flatten()
			.find(|found| found["attributes"]["locale"] == locale)
			.and_then(|found| found["id"].as_str());
		if let Some(id) = existing {
			self.send(
				"PATCH",
				&format!("/v1/appStoreVersionLocalizations/{id}"),
				Some(&json!({ "data": {
					"type": "appStoreVersionLocalizations", "id": id, "attributes": attributes,
				}})),
			)?
			.ok("setting the version's text")?;
		} else {
			let mut attributes = attributes.clone();
			attributes["locale"] = json!(locale);
			self.send(
				"POST",
				"/v1/appStoreVersionLocalizations",
				Some(&json!({ "data": {
					"type": "appStoreVersionLocalizations",
					"attributes": attributes,
					"relationships": { "appStoreVersion": {
						"data": { "type": "appStoreVersions", "id": version },
					}},
				}})),
			)?
			.ok("adding the version's text")?;
		}
		Ok(())
	}

	/// Sets the privacy policy's address in App Information's `en-US`
	/// localization, on the app information being prepared. Returns whether
	/// there was one to set it on.
	///
	/// # Errors
	///
	/// When App Store Connect refuses.
	pub fn privacy_policy(&self, app: &str, url: &str) -> Result<bool> {
		let infos = self.json(
			"GET",
			&format!("/v1/apps/{app}/appInfos"),
			None,
			"reading the app's information",
		)?;
		let infos = infos["data"].as_array().cloned().unwrap_or_default();
		// The one in preparation, when a version is; the live one otherwise.
		let info = infos
			.iter()
			.find(|info| {
				let state = info["attributes"]["appStoreState"]
					.as_str()
					.or_else(|| info["attributes"]["state"].as_str())
					.unwrap_or_default();
				EDITABLE.contains(&state) || state == "PREPARE_FOR_SUBMISSION"
			})
			.or_else(|| infos.first())
			.and_then(|info| info["id"].as_str());
		let Some(info) = info else {
			return Ok(false);
		};
		let localizations = self.json(
			"GET",
			&format!("/v1/appInfos/{info}/appInfoLocalizations"),
			None,
			"reading App Information's localizations",
		)?;
		let Some(id) = localizations["data"]
			.as_array()
			.into_iter()
			.flatten()
			.find(|found| found["attributes"]["locale"] == "en-US")
			.and_then(|found| found["id"].as_str())
		else {
			return Ok(false);
		};
		self.send(
			"PATCH",
			&format!("/v1/appInfoLocalizations/{id}"),
			Some(&json!({ "data": {
				"type": "appInfoLocalizations", "id": id,
				"attributes": { "privacyPolicyUrl": url },
			}})),
		)?
		.ok("setting the privacy policy's address")?;
		Ok(true)
	}

	/// Attaches the build to the version.
	///
	/// # Errors
	///
	/// When App Store Connect refuses.
	pub fn attach(&self, version: &str, build: &str) -> Result<()> {
		self.send(
			"PATCH",
			&format!("/v1/appStoreVersions/{version}/relationships/build"),
			Some(&json!({ "data": { "type": "builds", "id": build } })),
		)?
		.ok("attaching the build to the App Store version")
		.map(drop)
	}

	/// Sends the version to App Review, and returns the submission's state.
	///
	/// # Errors
	///
	/// When App Store Connect refuses, such as when the listing or the app's
	/// privacy answers are incomplete.
	pub fn submit_for_review(&self, app: &str, version: &str) -> Result<String> {
		let open = self.json(
			"GET",
			&format!(
				"/v1/reviewSubmissions?{}={app}&{}=IOS&{}=READY_FOR_REVIEW",
				filter("filter[app]"),
				filter("filter[platform]"),
				filter("filter[state]")
			),
			None,
			"reading the app's review submissions",
		)?;
		let submission = match open["data"].get(0).and_then(|found| found["id"].as_str()) {
			Some(id) => id.to_owned(),
			None => {
				self.json(
					"POST",
					"/v1/reviewSubmissions",
					Some(&json!({ "data": {
						"type": "reviewSubmissions",
						"attributes": { "platform": "IOS" },
						"relationships": { "app": { "data": { "type": "apps", "id": app } } },
					}})),
					"creating a review submission",
				)?["data"]["id"]
					.as_str()
					.context("App Store Connect created a submission without an ID")?
					.to_owned()
			}
		};
		let item = self.send(
			"POST",
			"/v1/reviewSubmissionItems",
			Some(&json!({ "data": {
				"type": "reviewSubmissionItems",
				"relationships": {
					"reviewSubmission": { "data": { "type": "reviewSubmissions", "id": submission } },
					"appStoreVersion": { "data": { "type": "appStoreVersions", "id": version } },
				},
			}})),
		)?;
		// 409: the version is in this submission already, from an earlier run.
		if item.status != 409 {
			item.ok("adding the version to the review submission")?;
		}
		let submitted = self.json(
			"PATCH",
			&format!("/v1/reviewSubmissions/{submission}"),
			Some(&json!({ "data": {
				"type": "reviewSubmissions", "id": submission,
				"attributes": { "submitted": true },
			}})),
			"submitting for review",
		)?;
		Ok(submitted["data"]["attributes"]["state"]
			.as_str()
			.unwrap_or("WAITING_FOR_REVIEW")
			.to_lowercase()
			.replace('_', " "))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn key() -> KeyFile {
		KeyFile {
			path: PathBuf::from("/keys/AuthKey_KEY123.p8"),
			_temporary: None,
			id: Secret::new("KEY123".into()),
			issuer: Secret::new("issuer-1".into()),
		}
	}

	#[test]
	fn xcode_archives_and_uploads_with_the_api_key() {
		let archive = archive(
			Path::new("/app"),
			Path::new("/app/.leptoskit/publish/ios"),
			"ABCDE12345",
			"1.4.0",
			42,
			&key(),
		);
		assert_eq!(archive.program, "xcodebuild");
		assert_eq!(archive.dir.as_deref(), Some(Path::new("/app")));
		assert_eq!(archive.unset, ["LD", "LDPLUSPLUS"]);
		insta::assert_snapshot!(archive.args.join(" "), @"archive -project .leptoskit/ios/Host.xcodeproj -scheme App -configuration Release -destination generic/platform=iOS -archivePath /app/.leptoskit/publish/ios/App.xcarchive -derivedDataPath .leptoskit/ios/build -allowProvisioningUpdates -authenticationKeyPath /keys/AuthKey_KEY123.p8 -authenticationKeyID KEY123 -authenticationKeyIssuerID issuer-1 DEVELOPMENT_TEAM=ABCDE12345 CODE_SIGN_STYLE=Automatic MARKETING_VERSION=1.4.0 CURRENT_PROJECT_VERSION=42");
		let export = export(Path::new("/app/.leptoskit/publish/ios"), &key());
		insta::assert_snapshot!(export.args.join(" "), @"-exportArchive -archivePath /app/.leptoskit/publish/ios/App.xcarchive -exportPath /app/.leptoskit/publish/ios/export -exportOptionsPlist /app/.leptoskit/publish/ios/ExportOptions.plist -allowProvisioningUpdates -authenticationKeyPath /keys/AuthKey_KEY123.p8 -authenticationKeyID KEY123 -authenticationKeyIssuerID issuer-1");
	}

	#[test]
	fn export_options_upload_to_app_store_connect() {
		let options = export_options("ABCDE12345");
		let parsed = plist::Value::from_reader_xml(options.as_bytes()).unwrap();
		let parsed = parsed.as_dictionary().unwrap();
		assert_eq!(
			parsed.get("method").and_then(plist::Value::as_string),
			Some("app-store-connect")
		);
		assert_eq!(
			parsed.get("destination").and_then(plist::Value::as_string),
			Some("upload")
		);
		assert_eq!(
			parsed.get("teamID").and_then(plist::Value::as_string),
			Some("ABCDE12345")
		);
		assert_eq!(
			parsed
				.get("manageAppVersionAndBuildNumber")
				.and_then(plist::Value::as_boolean),
			Some(false)
		);
	}

	#[test]
	fn a_key_given_as_text_is_written_privately_and_removed() {
		let dir = tempfile::tempdir().unwrap();
		let key = Key::new(
			Secret::new("issuer".into()),
			Secret::new("KEY1".into()),
			&Secret::new("-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n".into()),
		)
		.unwrap();
		let file = key.file(dir.path()).unwrap();
		let path = file.path.clone();
		assert!(path.ends_with("AuthKey_KEY1.p8"));
		assert!(
			std::fs::read_to_string(&path)
				.unwrap()
				.contains("BEGIN PRIVATE KEY")
		);
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt as _;
			assert_eq!(
				std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
				0o600
			);
		}
		drop(file);
		assert!(!path.exists());
	}
}
