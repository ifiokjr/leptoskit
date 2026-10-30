//! The store listing kept in the project, under `store/`, so it is reviewed
//! like code: what the stores' APIs accept is uploaded, and the rest goes
//! into the steps, ready to paste.
//!
//! ```text
//! store/
//! ├── listing.toml        privacy_policy, homepage, support, category
//! ├── permissions.toml    one sentence per permission, for reviewers
//! └── en-US/
//!     ├── description.md
//!     └── release_notes.md
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
