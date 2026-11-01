//! Google Play, through the Play Developer API's edits with a service
//! account, for the Android App Bundle Gradle builds and signs with the
//! upload key.

use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use serde_json::Value;
use serde_json::json;

use super::http::Body;
use super::http::Http;
use super::http::Reply;
use super::images::Image;
use super::secrets::Secret;
use super::tools::Tool;

/// The API's address.
pub const API: &str = "https://androidpublisher.googleapis.com";
/// The credential: the service account's JSON key, or the path to it.
pub const CREDENTIAL: &str = "LEPTOSKIT_PUBLISH_PLAY_SERVICE_ACCOUNT";
/// Where to make it.
pub const HOW: &str = "create a Google Cloud service account with a JSON key, turn on the Google \
                       Play Android Developer API in its project, then invite its email in Play \
                       Console under Users and permissions, with permission to release the app";
/// The upload key, which Gradle signs the bundle with.
pub const KEYSTORE: &str = "LEPTOSKIT_ANDROID_KEYSTORE";
pub const KEYSTORE_PASSWORD: &str = "LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD";
pub const KEY_ALIAS: &str = "LEPTOSKIT_ANDROID_KEY_ALIAS";
pub const KEY_PASSWORD: &str = "LEPTOSKIT_ANDROID_KEY_PASSWORD";
/// Where to make it.
pub const KEYSTORE_HOW: &str = "make an upload key once with `keytool -genkeypair -keystore \
                                upload.jks -alias upload -keyalg RSA -keysize 2048 -validity \
                                10000`, keep it outside the project, and set its absolute path, \
                                its two passwords, and its alias";
/// The lowest `targetSdk` Google Play takes for new apps and updates, since
/// 31 August 2026.
pub const TARGET_SDK: u32 = 36;
/// The longest release notes Google Play takes, per language.
pub const NOTES_LIMIT: usize = 500;
/// Where Gradle writes the release bundle, under the Android host.
pub const BUNDLE: &str = "app/build/outputs/bundle/release/app-release.aab";

/// What Gradle signs the bundle with.
pub struct Signing {
	pub keystore: Secret,
	pub keystore_password: Secret,
	pub key_alias: Secret,
	pub key_password: Secret,
}

/// `gradlew bundleRelease` in the Android host, numbered `version_code` and
/// signed with the upload key, which reaches Gradle only through its
/// environment.
#[must_use]
pub fn bundle(android: &Path, version_code: u64, signing: &Signing) -> Tool {
	Tool::new(
		&android
			.join(if cfg!(windows) {
				"gradlew.bat"
			} else {
				"gradlew"
			})
			.display()
			.to_string(),
	)
	.dir(android)
	.args(["--no-daemon", "bundleRelease"])
	.arg(format!("-Pleptoskit.versionCode={version_code}"))
	.env(KEYSTORE, signing.keystore.clone())
	.env(KEYSTORE_PASSWORD, signing.keystore_password.clone())
	.env(KEY_ALIAS, signing.key_alias.clone())
	.env(KEY_PASSWORD, signing.key_password.clone())
}

/// The Gradle script's `targetSdk`, when it sets one as a number.
fn target_sdk(script: &str) -> Option<u32> {
	let after = script.split("targetSdk").nth(1)?;
	let after = after.trim_start().strip_prefix('=')?.trim_start();
	after
		.split(|character: char| !character.is_ascii_digit())
		.next()?
		.parse()
		.ok()
}

/// What keeps the app's Gradle script from making a bundle Google Play
/// takes: the build number, the upload key, and the target SDK.
#[must_use]
pub fn problems(script: &str, path: &Path) -> Vec<String> {
	let mut problems = Vec::new();
	if !script.contains("leptoskit.versionCode") {
		problems.push(format!(
			"{} doesn't take the build number leptoskit chooses: set versionCode from the \
			 `leptoskit.versionCode` property, as `leptoskit new` writes it",
			path.display()
		));
	}
	if !script.contains(KEYSTORE) {
		problems.push(format!(
			"{} doesn't sign the release with the upload key: add the signingConfigs block that \
			 reads {KEYSTORE}, as `leptoskit new` writes it",
			path.display()
		));
	}
	match target_sdk(script) {
		Some(target) if target >= TARGET_SDK => {}
		Some(target) => {
			problems.push(format!(
				"{} targets API {target}; Google Play takes new apps and updates only from API \
				 {TARGET_SDK}: raise targetSdk and compileSdk",
				path.display()
			));
		}
		None => {
			problems.push(format!(
				"{} sets no targetSdk; Google Play needs API {TARGET_SDK} or later",
				path.display()
			));
		}
	}
	problems
}

/// How a release reaches its track's users.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rollout {
	/// Held in Play Console until it is released there or with `--submit`.
	Draft,
	/// To every user of the track.
	Everyone,
	/// To this fraction of the track's users first.
	Staged(f64),
}

/// A track's release of one bundle, as the edits API takes it.
#[must_use]
pub fn release(name: &str, version_code: u64, rollout: Rollout, notes: Option<&str>) -> Value {
	let mut release = json!({
		"name": name,
		"versionCodes": [version_code.to_string()],
		"status": match rollout {
			Rollout::Draft => "draft",
			Rollout::Everyone => "completed",
			Rollout::Staged(_) => "inProgress",
		},
	});
	if let Rollout::Staged(fraction) = rollout {
		release["userFraction"] = json!(fraction);
	}
	if let Some(notes) = notes {
		release["releaseNotes"] = json!([{ "language": "en-US", "text": notes }]);
	}
	release
}

/// A signed-in client for one app.
pub struct Play<'a> {
	http: &'a Http,
	api: String,
	package: String,
	token: Secret,
}

/// One edit: changes to the app that take effect together when committed.
pub struct Edit<'p, 'a> {
	play: &'p Play<'a>,
	id: String,
}

impl<'a> Play<'a> {
	/// Signs in with the service account.
	///
	/// # Errors
	///
	/// When the key is unusable or Google refuses it.
	pub fn connect(
		http: &'a Http,
		api: &str,
		oauth: &str,
		account: &Secret,
		package: &str,
	) -> Result<Self> {
		let token = super::google::access_token(
			http,
			oauth,
			account,
			CREDENTIAL,
			"https://www.googleapis.com/auth/androidpublisher",
			"Google Play",
		)?;
		Ok(Self {
			http,
			api: api.trim_end_matches('/').to_owned(),
			package: package.to_owned(),
			token,
		})
	}

	fn send(&self, method: &str, url: &str, body: Option<Body>) -> Result<Reply> {
		let authorization = format!("Bearer {}", self.token.expose());
		let reply = self
			.http
			.send(method, url, &[("authorization", &authorization)], body)?;
		if reply.status == 403 {
			anyhow::bail!(
				"Google Play refused the service account for {}: invite its email in Play Console \
				 under Users and permissions, with permission to release the app",
				self.package
			);
		}
		Ok(reply)
	}

	fn app(&self) -> String {
		format!(
			"{}/androidpublisher/v3/applications/{}",
			self.api, self.package
		)
	}

	/// Opens an edit, or `None` when Google Play has no such app yet, which
	/// happens until its first bundle is uploaded in Play Console.
	///
	/// # Errors
	///
	/// When Google Play refuses.
	pub fn edit(&self) -> Result<Option<Edit<'_, 'a>>> {
		let reply = self.send(
			"POST",
			&format!("{}/edits", self.app()),
			Some(Body::json(&json!({}))),
		)?;
		if reply.status == 404 {
			return Ok(None);
		}
		let edit = reply.ok("opening an edit on Google Play")?.json()?;
		Ok(Some(Edit {
			play: self,
			id: edit["id"]
				.as_str()
				.context("Google Play opened an edit without an ID")?
				.to_owned(),
		}))
	}
}

impl Edit<'_, '_> {
	fn url(&self, path: &str) -> String {
		format!("{}/edits/{}{path}", self.play.app(), self.id)
	}

	/// The highest version code on any track, or uploaded to none.
	///
	/// # Errors
	///
	/// When Google Play refuses.
	pub fn highest_version_code(&self) -> Result<Option<u64>> {
		let tracks = self
			.play
			.send("GET", &self.url("/tracks"), None)?
			.ok("reading the app's tracks")?
			.json()?;
		let bundles = self
			.play
			.send("GET", &self.url("/bundles"), None)?
			.ok("reading the app's bundles")?
			.json()?;
		let on_tracks = tracks["tracks"]
			.as_array()
			.into_iter()
			.flatten()
			.flat_map(|track| track["releases"].as_array().into_iter().flatten())
			.flat_map(|release| release["versionCodes"].as_array().into_iter().flatten())
			.filter_map(|code| code.as_str()?.parse::<u64>().ok());
		let uploaded = bundles["bundles"]
			.as_array()
			.into_iter()
			.flatten()
			.filter_map(|bundle| bundle["versionCode"].as_u64());
		Ok(on_tracks.chain(uploaded).max())
	}

	/// Uploads the App Bundle, and returns the version code Google Play read
	/// from it.
	///
	/// # Errors
	///
	/// When Google Play refuses it, such as for a version code it has.
	pub fn upload(&self, aab: Vec<u8>) -> Result<u64> {
		let url = format!(
			"{}/upload/androidpublisher/v3/applications/{}/edits/{}/bundles?uploadType=media",
			self.play.api, self.play.package, self.id
		);
		let bundle = self
			.play
			.send("POST", &url, Some(Body::bytes(aab)))?
			.ok("uploading the App Bundle to Google Play")?
			.json()?;
		bundle["versionCode"]
			.as_u64()
			.context("Google Play took the bundle without a version code")
	}

	/// Sets `track`'s release.
	///
	/// # Errors
	///
	/// When Google Play refuses it.
	pub fn release(&self, track: &str, release: &Value) -> Result<()> {
		self.play
			.send(
				"PUT",
				&self.url(&format!("/tracks/{track}")),
				Some(Body::json(
					&json!({ "track": track, "releases": [release] }),
				)),
			)?
			.ok(&format!("setting the {track} track's release"))
			.map(drop)
	}

	/// Sets the store listing in `language`: the title and the short and
	/// full descriptions.
	///
	/// # Errors
	///
	/// When Google Play refuses the text.
	pub fn listing(&self, language: &str, title: &str, short: &str, full: &str) -> Result<()> {
		self.play
			.send(
				"PUT",
				&self.url(&format!("/listings/{language}")),
				Some(Body::json(&json!({
					"language": language,
					"title": title,
					"shortDescription": short,
					"fullDescription": full,
				}))),
			)?
			.ok("setting the store listing")
			.map(drop)
	}

	/// Replaces the listing's images of `image_type` in `language`, such as
	/// `phoneScreenshots` or `featureGraphic`: deletes them all, then
	/// uploads `images` in their order.
	///
	/// # Errors
	///
	/// When Google Play refuses an image.
	pub fn replace_images(&self, language: &str, image_type: &str, images: &[Image]) -> Result<()> {
		self.play
			.send(
				"DELETE",
				&self.url(&format!("/listings/{language}/{image_type}")),
				None,
			)?
			.ok(&format!("deleting the listing's {image_type}"))?;
		for image in images {
			let url = format!(
				"{}/upload/androidpublisher/v3/applications/{}/edits/{}/listings/{language}/\
				 {image_type}?uploadType=media",
				self.play.api, self.play.package, self.id
			);
			self.play
				.send(
					"POST",
					&url,
					Some(Body::typed(image.content_type, image.bytes.clone())),
				)?
				.ok(&format!("uploading {} to Google Play", image.name))?;
		}
		Ok(())
	}

	/// Commits the edit's changes.
	///
	/// # Errors
	///
	/// When Google Play refuses them, with its reason.
	pub fn commit(self) -> Result<()> {
		self.play
			.send("POST", &self.url(":commit"), None)?
			.ok("committing the edit on Google Play")
			.map(drop)
	}

	/// Drops the edit's changes.
	pub fn discard(self) {
		let _ = self.play.send("DELETE", &self.url(""), None);
	}
}

/// The release bundle Gradle wrote.
#[must_use]
pub fn bundle_path(android: &Path) -> PathBuf {
	android.join(BUNDLE)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn gradle_bundles_with_the_build_number_and_the_upload_key() {
		let signing = Signing {
			keystore: Secret::new("/keys/upload.jks".into()),
			keystore_password: Secret::new("store-pass".into()),
			key_alias: Secret::new("upload".into()),
			key_password: Secret::new("key-pass".into()),
		};
		let tool = bundle(Path::new("/app/android"), 42, &signing);
		assert!(tool.program.ends_with("gradlew") || tool.program.ends_with("gradlew.bat"));
		assert_eq!(tool.dir.as_deref(), Some(Path::new("/app/android")));
		assert_eq!(
			tool.args,
			["--no-daemon", "bundleRelease", "-Pleptoskit.versionCode=42"]
		);
		let names: Vec<&str> = tool.env.iter().map(|(name, _)| *name).collect();
		assert_eq!(
			names,
			[KEYSTORE, KEYSTORE_PASSWORD, KEY_ALIAS, KEY_PASSWORD]
		);
		// The passwords never print with the tool.
		assert!(!format!("{tool:?}").contains("store-pass"));
	}

	#[test]
	fn the_template_makes_a_bundle_google_play_takes() {
		let template = include_str!("../native/templates/build.gradle.kts");
		assert_eq!(target_sdk(template), Some(TARGET_SDK));
		assert_eq!(
			problems(template, Path::new("build.gradle.kts")),
			Vec::<String>::new()
		);

		let old =
			"android {\n    compileSdk = 35\n    defaultConfig {\n        targetSdk = 35\n        \
			 versionCode = 1\n    }\n}\n";
		let problems = problems(old, Path::new("android/app/build.gradle.kts"));
		assert_eq!(problems.len(), 3, "{problems:?}");
		assert!(problems[0].contains("leptoskit.versionCode"));
		assert!(problems[1].contains(KEYSTORE));
		assert!(problems[2].contains("targets API 35"));
	}

	#[test]
	fn releases_are_drafts_until_submitted() {
		assert_eq!(
			release("1.4.0", 42, Rollout::Draft, None),
			json!({ "name": "1.4.0", "versionCodes": ["42"], "status": "draft" })
		);
		assert_eq!(
			release("1.4.0", 42, Rollout::Staged(0.1), Some("Faster sync.")),
			json!({
				"name": "1.4.0", "versionCodes": ["42"], "status": "inProgress", "userFraction": 0.1,
				"releaseNotes": [{ "language": "en-US", "text": "Faster sync." }],
			})
		);
		assert_eq!(
			release("1.4.0", 42, Rollout::Everyone, None)["status"],
			"completed"
		);
	}
}
