//! The store listing kept in the project, under `store/`, so it is reviewed
//! like code: what the stores' APIs accept is uploaded, and the rest goes
//! into the steps, ready to paste.
//!
//! ```text
//! store/
//! ├── listing.toml        name, keywords, privacy_policy, homepage, support, category
//! ├── permissions.toml    one sentence per permission, for reviewers
//! ├── en-US/
//! │   ├── summary.md      one or two sentences: Play's short description, AMO's summary
//! │   ├── description.md
//! │   └── release_notes.md
//! └── screenshots/
//!     └── screenshots.toml  which screens `leptoskit publish screenshots` captures
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;

/// `store/listing.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Listing {
	/// The name the stores show, when it isn't the app's own.
	pub name: Option<String>,
	/// Search keywords, for the App Store.
	pub keywords: Vec<String>,
	/// The privacy policy's address, which stores require when an
	/// extension handles personal or website data.
	pub privacy_policy: Option<String>,
	/// The app's website.
	pub homepage: Option<String>,
	/// Where people get help.
	pub support: Option<String>,
	/// The store category, such as `Productivity`.
	pub category: Option<String>,
}

/// Everything in `store/`.
#[derive(Debug, Clone, Default)]
pub struct Store {
	pub listing: Listing,
	/// Each permission's justification, by permission.
	pub permissions: BTreeMap<String, String>,
	/// The `en-US` summary, if written: a sentence or two.
	pub summary: Option<String>,
	/// The `en-US` description, if written.
	pub description: Option<String>,
	/// The `en-US` release notes, if written.
	pub release_notes: Option<String>,
}

impl Store {
	/// Reads `store/` under `root`; every file is optional.
	///
	/// # Errors
	///
	/// When a file exists but can't be parsed.
	pub fn load(root: &Path) -> Result<Self> {
		let dir = root.join("store");
		let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
		let listing = match read("listing.toml") {
			Some(text) => toml::from_str(&text).context("could not parse store/listing.toml")?,
			None => Listing::default(),
		};
		let permissions = match read("permissions.toml") {
			Some(text) => {
				toml::from_str(&text).context(
					"could not parse store/permissions.toml; write one `permission = \
					 \"sentence\"` line each",
				)?
			}
			None => BTreeMap::new(),
		};
		let trimmed = |text: Option<String>| {
			text.map(|text| text.trim().to_owned())
				.filter(|text| !text.is_empty())
		};
		Ok(Self {
			listing,
			permissions,
			summary: trimmed(read("en-US/summary.md")),
			description: trimmed(read("en-US/description.md")),
			release_notes: trimmed(read("en-US/release_notes.md")),
		})
	}

	/// A permission's justification, or what to write.
	#[must_use]
	pub fn justification(&self, permission: &str) -> String {
		self.permissions
			.get(permission)
			.cloned()
			.unwrap_or_else(|| {
				format!("(write one sentence for `{permission}` in store/permissions.toml)")
			})
	}
}

/// The longest text each store takes, in characters (bytes for the App
/// Store's keywords).
pub mod limits {
	pub const APP_STORE_DESCRIPTION: usize = 4000;
	pub const APP_STORE_KEYWORDS: usize = 100;
	pub const APP_STORE_PROMOTIONAL: usize = 170;
	pub const PLAY_TITLE: usize = 30;
	pub const PLAY_SHORT: usize = 80;
	pub const PLAY_FULL: usize = 4000;
	pub const AMO_SUMMARY: usize = 250;
}

impl Store {
	/// The App Store's keywords, comma-separated as it takes them.
	#[must_use]
	pub fn keywords(&self) -> Option<String> {
		(!self.listing.keywords.is_empty()).then(|| self.listing.keywords.join(","))
	}

	/// What in `store/` is too long for `store`, as `leptoskit publish`
	/// names it.
	#[must_use]
	pub fn too_long(&self, store: &str, name: &str) -> Vec<String> {
		use limits::*;

		let mut problems = Vec::new();
		let mut check = |what: &str, text: Option<&str>, limit: usize, bytes: bool| {
			if let Some(text) = text {
				let length = if bytes {
					text.len()
				} else {
					text.chars().count()
				};
				if length > limit {
					problems.push(format!(
						"{what} has {length} {}; {} takes at most {limit}",
						if bytes { "bytes" } else { "characters" },
						store_name(store)
					));
				}
			}
		};
		let description = self.description.as_deref();
		let summary = self.summary.as_deref();
		match store {
			"app_store" => {
				check(
					"store/en-US/description.md",
					description,
					APP_STORE_DESCRIPTION,
					false,
				);
				check(
					"the keywords in store/listing.toml",
					self.keywords().as_deref(),
					APP_STORE_KEYWORDS,
					true,
				);
				check(
					"store/en-US/summary.md",
					summary,
					APP_STORE_PROMOTIONAL,
					false,
				);
			}
			"play" => {
				check(
					"the store name",
					Some(self.listing.name.as_deref().unwrap_or(name)),
					PLAY_TITLE,
					false,
				);
				check("store/en-US/summary.md", summary, PLAY_SHORT, false);
				check("store/en-US/description.md", description, PLAY_FULL, false);
			}
			"firefox" => check("store/en-US/summary.md", summary, AMO_SUMMARY, false),
			_ => {}
		}
		problems
	}
}

fn store_name(store: &str) -> &'static str {
	match store {
		"app_store" => "the App Store",
		"play" => "Google Play",
		"firefox" => "addons.mozilla.org",
		_ => "the store",
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn text_too_long_for_a_store_is_named() {
		let store = Store {
			listing: Listing {
				keywords: vec!["notes".into(), "research".into()],
				..Listing::default()
			},
			summary: Some("x".repeat(81)),
			..Store::default()
		};
		assert_eq!(store.keywords().as_deref(), Some("notes,research"));
		assert_eq!(store.too_long("app_store", "Notes"), Vec::<String>::new());
		assert_eq!(
			store.too_long("play", "Notes"),
			["store/en-US/summary.md has 81 characters; Google Play takes at most 80"]
		);
		assert_eq!(
			store.too_long("play", "A name far too long for Google Play"),
			[
				"the store name has 35 characters; Google Play takes at most 30",
				"store/en-US/summary.md has 81 characters; Google Play takes at most 80",
			]
		);
	}
}
