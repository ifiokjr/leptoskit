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
pub const STORES: [&str; 4] = ["chrome", "edge", "firefox", "app_store"];

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
		if let Some(chrome) = &self.chrome {
			let item = chrome.item_id.len() == 32
				&& chrome
					.item_id
					.bytes()
					.all(|byte| (b'a'..=b'p').contains(&byte));
			if !item {
				return Some(format!(
					"publish.chrome.item_id `{}` must be the item's 32-letter ID, from its URL in \
					 the developer dashboard",
					chrome.item_id
				));
			}
			if chrome.publisher_id.trim().is_empty() {
				return Some("publish.chrome.publisher_id can't be empty".into());
			}
		}
		if let Some(edge) = &self.edge
			&& edge.product_id.trim().is_empty()
		{
			return Some("publish.edge.product_id can't be empty".into());
		}
		if self.firefox.is_some() && !has_firefox_id {
			return Some("publish.firefox needs [extension.firefox] with the add-on's id".into());
		}
		if let Some(problem) = self.app_store.as_ref().and_then(|store| store.problem(app)) {
			return Some(problem);
		}
		None
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
