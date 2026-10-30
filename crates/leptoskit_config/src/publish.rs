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
pub const STORES: [&str; 3] = ["chrome", "edge", "firefox"];

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
		stores
	}

	/// The first problem with this configuration, in words.
	#[must_use]
	pub fn problem(&self, has_extension: bool, has_firefox_id: bool) -> Option<String> {
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
		None
	}
}
