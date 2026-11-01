//! `[publish]`: the stores `leptoskit publish` uploads to.
//!
//! Each table names what a store needs to find the app; credentials never go
//! here, only in the environment or a credentials file outside the project.

use serde::Deserialize;
use serde::Serialize;

/// The stores an app is published to.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct PublishConfig {
	/// The Chrome Web Store, for the Chromium package.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub chrome: Option<ChromeStore>,
	/// Microsoft Edge Add-ons, for the Chromium package.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub edge: Option<EdgeStore>,
	/// addons.mozilla.org, for the Firefox package.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub firefox: Option<FirefoxStore>,
	/// App Store Connect, for the iOS app and the Safari extension inside
	/// it: TestFlight, then App Store review.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub app_store: Option<AppStore>,
	/// The macOS app, signed with a Developer ID and notarized, for download
	/// outside the Mac App Store.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub macos: Option<MacosStore>,
	/// Google Play, for the Android App Bundle.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub play: Option<PlayStore>,
}

/// An app in Google Play Console, created once there. Its package name is
/// `native.package_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct PlayStore {
	/// The track each upload goes to: `internal`, `alpha`, `beta`,
	/// `production`, or a closed testing track's name.
	pub track: String,
	/// With `--submit` on `production`, the fraction of users who get the
	/// release first, such as `0.1`; the rest get it when the rollout is
	/// raised in Play Console. Without it, everyone gets it.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub rollout: Option<f64>,
}

impl Default for PlayStore {
	fn default() -> Self {
		Self {
			track: "internal".to_owned(),
			rollout: None,
		}
	}
}

/// How the macOS app is distributed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum MacosDistribution {
	/// Signed with a Developer ID certificate and notarized by Apple, so
	/// Gatekeeper opens it on any Mac.
	#[default]
	DeveloperId,
}

/// The notarized macOS download.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct MacosStore {
	/// `developer_id`.
	pub distribution: MacosDistribution,
	/// A GitHub repository, `owner/name`, whose release `v<app.version>`
	/// gets the `.dmg` and `.zip` with `--submit`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub github_release: Option<String>,
}

/// An app in App Store Connect, created once there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AppStore {
	/// The app's Apple ID: the number in its App Store Connect address,
	/// under App Information.
	#[cfg_attr(feature = "schema", schemars(regex(pattern = r"^[0-9]+$")))]
	pub app_id: String,
	/// The Apple Developer team that signs the app, such as `ABCDE12345`.
	/// Defaults to `native.apple_team_id`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	#[cfg_attr(feature = "schema", schemars(regex(pattern = r"^[A-Z0-9]{10}$")))]
	pub team_id: Option<String>,
	/// The TestFlight groups each uploaded build is added to, by name.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub testflight_groups: Vec<String>,
}

/// An item in the Chrome Web Store, created once in its developer
/// dashboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ChromeStore {
	/// The publisher's ID, from the developer dashboard's account page.
	pub publisher_id: String,
	/// The item's 32-letter ID.
	pub item_id: String,
}

/// A product in Microsoft Partner Center, created once there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct EdgeStore {
	/// The product ID, from the product's Extension identity section. It
	/// isn't the public Store ID.
	pub product_id: String,
}

/// Where a Firefox add-on is distributed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum FirefoxChannel {
	/// Listed on addons.mozilla.org, after review.
	#[default]
	Listed,
	/// Signed for you to distribute yourself.
	Unlisted,
}

impl FirefoxChannel {
	/// The channel's name in the API.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Listed => "listed",
			Self::Unlisted => "unlisted",
		}
	}
}

/// An add-on on addons.mozilla.org. Its ID is `[extension.firefox].id`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct FirefoxStore {
	/// `listed` or `unlisted`.
	pub channel: FirefoxChannel,
}

/// The stores `leptoskit publish` knows, as the command line names them.
pub const STORES: [&str; 6] = ["chrome", "edge", "firefox", "app_store", "macos", "play"];

impl PublishConfig {
	/// Whether no store is configured.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.stores().is_empty()
	}

	/// The configured stores, in the order uploads happen.
	#[must_use]
	pub fn stores(&self) -> Vec<&'static str> {
		let mut stores = Vec::new();

		if self.chrome.is_some() {
			stores.push("chrome");
		}

		if self.edge.is_some() {
			stores.push("edge");
		}

		if self.firefox.is_some() {
			stores.push("firefox");
		}

		if self.app_store.is_some() {
			stores.push("app_store");
		}

		if self.macos.is_some() {
			stores.push("macos");
		}

		if self.play.is_some() {
			stores.push("play");
		}

		stores
	}

	/// The first problem with this configuration, in words.
	#[must_use]
	pub fn problem(&self, app: &Facts) -> Option<String> {
		let Facts {
			has_extension,
			has_firefox_id,
			..
		} = *app;

		if !has_extension
			&& (self.chrome.is_some() || self.edge.is_some() || self.firefox.is_some())
		{
			return Some(
				"the extension stores need an [extension] table and `extension` in app.platforms"
					.into(),
			);
		}

		if let Some(problem) = self.chrome.as_ref().and_then(chrome_problem) {
			return Some(problem);
		}

		if let Some(problem) = self.edge.as_ref().and_then(edge_problem) {
			return Some(problem);
		}

		if self.firefox.is_some() && !has_firefox_id {
			return Some("publish.firefox needs [extension.firefox] with the add-on's id".into());
		}

		if let Some(problem) = self.app_store.as_ref().and_then(|store| store.problem(app)) {
			return Some(problem);
		}

		if let Some(problem) = self
			.macos
			.as_ref()
			.and_then(|macos| macos_problem(macos, app))
		{
			return Some(problem);
		}

		self.play.as_ref().and_then(|play| play_problem(play, app))
	}
}

/// What `[publish]`'s validation needs to know about the rest of the app.
#[derive(Debug, Clone, Copy, Default)]
pub struct Facts<'a> {
	/// `app.platforms` has `extension`, and there is an `[extension]` table.
	pub has_extension: bool,
	/// `[extension.firefox]` names the add-on.
	pub has_firefox_id: bool,
	/// `app.platforms`.
	pub platforms: &'a [crate::Platform],
	/// `native.apple_team_id`.
	pub apple_team_id: Option<&'a str>,
}

impl AppStore {
	/// The team that signs the app: this table's, or `native.apple_team_id`.
	#[must_use]
	pub fn team<'a>(&'a self, apple_team_id: Option<&'a str>) -> Option<&'a str> {
		self.team_id.as_deref().or(apple_team_id)
	}

	fn problem(&self, app: &Facts) -> Option<String> {
		if !app.platforms.contains(&crate::Platform::Ios) {
			return Some("publish.app_store needs `ios` in app.platforms".into());
		}

		if self.app_id.is_empty() || !self.app_id.bytes().all(|byte| byte.is_ascii_digit()) {
			return Some(format!(
				"publish.app_store.app_id `{}` must be the app's Apple ID, the number under App \
				 Information in App Store Connect",
				self.app_id
			));
		}

		match self.team(app.apple_team_id) {
			None => {
				Some(
					"publish.app_store needs team_id, or native.apple_team_id: the 10-character \
					 Apple Developer team ID"
						.into(),
				)
			}
			Some(team)
				if team.len() != 10
					|| !team
						.bytes()
						.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()) =>
			{
				Some(format!(
					"the Apple team ID `{team}` must be 10 capital letters and digits, from the \
					 Membership page of developer.apple.com"
				))
			}
			Some(_) => {
				self.testflight_groups
					.iter()
					.any(|group| group.trim().is_empty())
					.then(|| "publish.app_store.testflight_groups can't name an empty group".into())
			}
		}
	}
}

/// The Chrome Web Store's item and publisher IDs.
fn chrome_problem(chrome: &ChromeStore) -> Option<String> {
	let item = chrome.item_id.len() == 32
		&& chrome
			.item_id
			.bytes()
			.all(|byte| (b'a'..=b'p').contains(&byte));

	if !item {
		return Some(format!(
			"publish.chrome.item_id `{}` must be the item's 32-letter ID, from its URL in the \
			 developer dashboard",
			chrome.item_id
		));
	}

	if chrome.publisher_id.trim().is_empty() {
		return Some("publish.chrome.publisher_id can't be empty".into());
	}

	None
}

/// The Edge Add-ons product ID.
fn edge_problem(edge: &EdgeStore) -> Option<String> {
	if edge.product_id.trim().is_empty() {
		return Some("publish.edge.product_id can't be empty".into());
	}

	None
}

/// macOS downloads: the platform, and GitHub's `owner/name`.
fn macos_problem(macos: &MacosStore, app: &Facts) -> Option<String> {
	if !app.platforms.contains(&crate::Platform::Macos) {
		return Some("publish.macos needs `macos` in app.platforms".into());
	}

	let repository = macos.github_release.as_ref()?;
	let parts: Vec<&str> = repository.split('/').collect();

	if parts.len() != 2 || parts.iter().any(|part| part.trim().is_empty()) {
		return Some(format!(
			"publish.macos.github_release `{repository}` must be a repository, `owner/name`"
		));
	}

	None
}

/// Google Play: the platform, track, and rollout.
fn play_problem(play: &PlayStore, app: &Facts) -> Option<String> {
	if !app.platforms.contains(&crate::Platform::Android) {
		return Some("publish.play needs `android` in app.platforms".into());
	}

	if play.track.trim().is_empty() {
		return Some("publish.play.track can't be empty".into());
	}

	if let Some(rollout) = play.rollout
		&& !(rollout > 0.0 && rollout < 1.0)
	{
		return Some(format!(
			"publish.play.rollout {rollout} must be a fraction between 0 and 1, such as 0.1; \
			 leave it out to release to everyone"
		));
	}

	None
}

#[cfg(test)]
mod tests {
	use std::path::Path;

	use crate::LeptoskitConfig;

	fn load(extra: &str) -> Result<LeptoskitConfig, String> {
		LeptoskitConfig::from_toml(
			&format!(
				"[app]\nname = \"notes\"\nplatforms = [\"web\", \"ios\"]\n[native]\npackage_id = \
				 \"com.example.notes\"\n{extra}"
			),
			Path::new("notes"),
		)
		.map_err(|error| error.to_string())
	}

	#[test]
	fn the_app_store_needs_an_app_id_and_a_team() {
		let config =
			load("[publish.app_store]\napp_id = \"1234567890\"\nteam_id = \"ABCDE12345\"\n")
				.unwrap();
		assert_eq!(config.publish.stores(), ["app_store"]);
		let store = config.publish.app_store.unwrap();
		assert_eq!(store.team(None), Some("ABCDE12345"));

		// The team falls back to the one the apps already sign with.
		let config =
			load("apple_team_id = \"ABCDE12345\"\n[publish.app_store]\napp_id = \"1234567890\"\n")
				.unwrap();
		assert!(config.publish.app_store.is_some());

		let error = load("[publish.app_store]\napp_id = \"1234567890\"\n").unwrap_err();
		assert!(error.contains("needs team_id"), "{error}");
		let error =
			load("[publish.app_store]\napp_id = \"com.example\"\nteam_id = \"ABCDE12345\"\n")
				.unwrap_err();
		assert!(error.contains("Apple ID"), "{error}");
		let error = load("[publish.app_store]\napp_id = \"1\"\nteam_id = \"abc\"\n").unwrap_err();
		assert!(error.contains("10 capital letters"), "{error}");
	}

	#[test]
	fn the_macos_download_needs_a_macos_app() {
		let toml = |platforms: &str, table: &str| {
			LeptoskitConfig::from_toml(
				&format!("[app]\nplatforms = {platforms}\n[publish.macos]\n{table}"),
				Path::new("notes"),
			)
			.map_err(|error| error.to_string())
		};
		let config = toml(
			"[\"web\", \"macos\"]",
			"github_release = \"example/notes\"\n",
		)
		.unwrap();
		assert_eq!(config.publish.stores(), ["macos"]);
		assert_eq!(
			config.publish.macos.unwrap().distribution,
			super::MacosDistribution::DeveloperId
		);
		let error = toml("[\"web\"]", "").unwrap_err();
		assert!(error.contains("`macos` in app.platforms"), "{error}");
		let error = toml("[\"web\", \"macos\"]", "github_release = \"notes\"\n").unwrap_err();
		assert!(error.contains("owner/name"), "{error}");
	}

	#[test]
	fn google_play_takes_a_track_and_a_rollout() {
		let toml = |table: &str| {
			LeptoskitConfig::from_toml(
				&format!("[app]\nplatforms = [\"web\", \"android\"]\n[publish.play]\n{table}"),
				Path::new("notes"),
			)
			.map_err(|error| error.to_string())
		};
		let config = toml("").unwrap();
		assert_eq!(config.publish.stores(), ["play"]);
		assert_eq!(config.publish.play.unwrap().track, "internal");
		let config = toml("track = \"production\"\nrollout = 0.1\n").unwrap();
		assert_eq!(config.publish.play.unwrap().rollout, Some(0.1));
		let error = toml("rollout = 1.0\n").unwrap_err();
		assert!(error.contains("between 0 and 1"), "{error}");
	}

	/// A config that builds an extension, so the extension stores apply.
	const EXTENSION: &str = r#"
[app]
name = "notes"
platforms = ["extension"]
default_platform = "extension"

[native]
package_id = "com.example.notes"

[extension]
browsers = ["chromium"]

[extension.devtools_page]
route = "/devtools"
panel = "/devtools/panel"
"#;

	#[test]
	fn the_extension_stores_check_their_ids() {
		let config = |extra: &str| {
			LeptoskitConfig::from_toml(&format!("{EXTENSION}{extra}"), Path::new("notes"))
				.map_err(|error| error.to_string())
		};

		// Chrome's item is 32 letters, `a` to `p`.
		let error = config("[publish.chrome]\nitem_id = \"short\"\npublisher_id = \"publisher\"\n")
			.unwrap_err();
		assert!(error.contains("32-letter ID"), "{error}");

		let error = config(
			"[publish.chrome]\nitem_id = \"abcdefghijklmnopabcdefghijklmnop\"\npublisher_id = \" \
			 \"\n",
		)
		.unwrap_err();
		assert!(error.contains("publisher_id"), "{error}");

		// Edge's product ID, and Firefox's add-on ID from
		// `[extension.firefox]`.
		let error = config("[publish.edge]\nproduct_id = \" \"\n").unwrap_err();
		assert!(error.contains("product_id"), "{error}");

		let error = config("[publish.firefox]\n").unwrap_err();
		assert!(error.contains("add-on's id"), "{error}");

		// A complete Chrome listing is accepted.
		let config = config(
			"[publish.chrome]\nitem_id = \"abcdefghijklmnopabcdefghijklmnop\"\npublisher_id = \
			 \"publisher\"\n",
		)
		.unwrap();
		assert_eq!(config.publish.stores(), ["chrome"]);
	}

	#[test]
	fn the_app_store_needs_an_ios_app() {
		let error = LeptoskitConfig::from_toml(
			"[app]\nplatforms = [\"web\"]\n[publish.app_store]\napp_id = \"1\"\nteam_id = \
			 \"ABCDE12345\"\n",
			Path::new("notes"),
		)
		.unwrap_err()
		.to_string();
		assert!(error.contains("`ios` in app.platforms"), "{error}");
	}
}
