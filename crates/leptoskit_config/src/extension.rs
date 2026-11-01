//! `[extension]`: the app as a browser extension.
//!
//! Each surface (the popup, the side panel, the options page, the new tab
//! page, a panel in the browser's developer tools, and content scripts) opens
//! one of the app's routes. The background worker is the only surface without
//! a screen.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::Serialize;

/// A family of browsers that share one extension package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Browser {
	/// Chrome, Edge, Brave, Opera, Arc, Vivaldi, and the other browsers built
	/// on Chromium: one package for every Chromium store.
	Chromium,
	/// Firefox, signed by addons.mozilla.org.
	Firefox,
	/// Safari on macOS and iOS, shipped inside the app's own native build.
	Safari,
}

impl Browser {
	/// Every family, in the order packages are built.
	pub const ALL: [Self; 3] = [Self::Chromium, Self::Firefox, Self::Safari];

	/// The family's name, as configuration and output folders spell it.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Chromium => "chromium",
			Self::Firefox => "firefox",
			Self::Safari => "safari",
		}
	}
}

impl std::fmt::Display for Browser {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str(self.name())
	}
}

/// The extension: its surfaces, permissions, and per-browser settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct ExtensionConfig {
	/// The name stores and the browser show; defaults to `app.name`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	/// One sentence about what the extension does, up to 132 characters,
	/// which the Chrome Web Store allows.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub description: Option<String>,
	/// The browser families to package for.
	pub browsers: Vec<Browser>,
	/// The server the extension's API calls reach, such as
	/// `https://api.example.com`. Its origin becomes a host permission.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub server: Option<String>,
	/// Extension permissions, added to those the app's plugins need.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub permissions: Vec<String>,
	/// Match patterns granted at install.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub host_permissions: Vec<String>,
	/// Match patterns the extension asks for at run time.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub optional_host_permissions: Vec<String>,
	/// The toolbar button's popup.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub popup: Option<ExtensionPage>,
	/// The side panel (Chromium) or sidebar (Firefox). Safari has none, so
	/// its package leaves it out.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub side_panel: Option<ExtensionPage>,
	/// The extension's settings page.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub options: Option<ExtensionPage>,
	/// A page that replaces the browser's new tab.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub new_tab: Option<ExtensionPage>,
	/// A panel in the browser's developer tools. Safari has no developer
	/// tools API, so its package leaves it out.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub devtools_page: Option<DevtoolsPage>,
	/// Interfaces shown on other sites' pages.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub content_scripts: Vec<ContentScript>,
	/// The background worker: the events it wakes for.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub background: Option<ExtensionBackground>,
	/// Keyboard shortcuts, by name.
	#[serde(skip_serializing_if = "BTreeMap::is_empty")]
	pub commands: BTreeMap<String, ExtensionCommand>,
	/// Firefox's settings.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub firefox: Option<FirefoxSettings>,
	/// Safari's settings.
	#[serde(skip_serializing_if = "SafariSettings::is_default")]
	pub safari: SafariSettings,
}

impl Default for ExtensionConfig {
	fn default() -> Self {
		Self {
			name: None,
			description: None,
			browsers: vec![Browser::Chromium, Browser::Firefox],
			server: None,
			permissions: Vec::new(),
			host_permissions: Vec::new(),
			optional_host_permissions: Vec::new(),
			popup: None,
			side_panel: None,
			options: None,
			new_tab: None,
			devtools_page: None,
			content_scripts: Vec::new(),
			background: None,
			commands: BTreeMap::new(),
			firefox: None,
			safari: SafariSettings::default(),
		}
	}
}

/// A page surface: the route it opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ExtensionPage {
	/// The app route the page opens, such as `/clip`.
	pub route: String,
	/// The families that get this page; every configured family when
	/// omitted.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub browsers: Option<Vec<Browser>>,
}

impl ExtensionPage {
	fn surface(&self, name: &'static str, setting: &'static str) -> PageSurface<'_> {
		PageSurface {
			name,
			setting,
			route: &self.route,
			browsers: self.browsers.as_deref(),
		}
	}
}

/// A panel in the browser's developer tools. The developer tools open
/// `route` out of sight each time they open for a tab, and that page adds the
/// panel, which shows `panel`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DevtoolsPage {
	/// The app route the developer tools open out of sight, such as
	/// `/devtools`. It adds the panel with
	/// `leptoskit::extension::devtools::create_panel` and shows nothing.
	pub route: String,
	/// The app route the panel shows, such as `/devtools/panel`.
	pub panel: String,
	/// The families that get the panel; every configured family but Safari,
	/// which has no developer tools API, when omitted.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub browsers: Option<Vec<Browser>>,
}

/// A page the package declares, from whichever surface configures it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageSurface<'a> {
	/// The surface's name in the package, which is also its files': `popup`
	/// is `popup.html` and `popup.js`.
	pub name: &'static str,
	/// The setting that names its route, such as `extension.popup.route`.
	pub setting: &'static str,
	/// The app route it opens.
	pub route: &'a str,
	/// The families it's limited to, if any.
	pub browsers: Option<&'a [Browser]>,
}

impl PageSurface<'_> {
	/// The table that configures the page, such as `extension.popup`.
	#[must_use]
	pub fn table(&self) -> &'static str {
		self.setting
			.rsplit_once('.')
			.map_or(self.setting, |(table, _)| table)
	}
}

/// When a content script reaches a page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Inject {
	/// Declared in the manifest, so it runs on every page its `matches`
	/// cover, which needs host permission for each.
	#[default]
	Manifest,
	/// Packaged, but injected only when the app asks, for the tab the user
	/// acted on. With `activeTab` it needs no host permission.
	OnRequest,
	/// Packaged, and registered at run time for the sites the user allows.
	Registered,
}

/// When a declared content script runs, relative to the page loading.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum RunAt {
	/// Before the page's own scripts.
	DocumentStart,
	/// After the document is parsed.
	DocumentEnd,
	/// Once the page is idle: the browsers' default.
	#[default]
	DocumentIdle,
}

impl RunAt {
	/// The manifest's spelling.
	#[must_use]
	pub const fn manifest(self) -> &'static str {
		match self {
			Self::DocumentStart => "document_start",
			Self::DocumentEnd => "document_end",
			Self::DocumentIdle => "document_idle",
		}
	}
}

/// An interface shown on other sites' pages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ContentScript {
	/// The script's name, which the app injects or registers it by:
	/// lowercase letters, digits, and dashes.
	pub name: String,
	/// The app route the script's interface opens.
	pub route: String,
	/// The pages it runs on. Required for `manifest` scripts.
	#[serde(default)]
	pub matches: Vec<String>,
	/// When it runs.
	#[serde(default)]
	pub run_at: RunAt,
	/// How it reaches a page.
	#[serde(default)]
	pub inject: Inject,
	/// The families that get this script; every configured family when
	/// omitted.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub browsers: Option<Vec<Browser>>,
}

/// The background worker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct ExtensionBackground {
	/// The events that wake it, such as `runtime.message` and `alarms`.
	pub events: Vec<String>,
}

/// The events a background worker can wake for.
pub const BACKGROUND_EVENTS: [&str; 9] = [
	"runtime.message",
	"runtime.installed",
	"runtime.startup",
	"runtime.connect",
	"alarms",
	"context_menus",
	"commands",
	"tabs.updated",
	"storage.changed",
];

/// A keyboard shortcut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ExtensionCommand {
	/// The keys suggested to the user, such as `Alt+Shift+C`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub suggested_key: Option<String>,
	/// What the shortcut does, as the browser's shortcut settings list it.
	pub description: String,
}

/// Firefox's settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct FirefoxSettings {
	/// The add-on's ID, such as `commonplace@example.com`, which
	/// addons.mozilla.org requires and keeps for good.
	pub id: String,
	/// The oldest Firefox that may install it; 121 by default.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub strict_min_version: Option<String>,
	/// The data the extension sends anywhere, in Firefox's categories, such
	/// as `websiteContent` and `personallyIdentifyingInfo`; `["none"]` when
	/// it sends nothing. addons.mozilla.org requires the declaration and
	/// Firefox shows it at install.
	#[serde(default = "no_data_collection")]
	pub data_collection: Vec<String>,
}

fn no_data_collection() -> Vec<String> {
	vec!["none".to_owned()]
}

/// The data categories Firefox's `data_collection_permissions` accepts.
pub const FIREFOX_DATA_COLLECTION: [&str; 12] = [
	"none",
	"authenticationInfo",
	"bookmarksInfo",
	"browsingActivity",
	"financialAndPaymentInfo",
	"healthInfo",
	"locationInfo",
	"personalCommunications",
	"personallyIdentifyingInfo",
	"searchTerms",
	"websiteActivity",
	"websiteContent",
];

/// Safari's settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct SafariSettings {
	/// Appended to `native.package_id` to make the Safari extension's bundle
	/// identifier, which Apple requires to start with the app's.
	pub bundle_id_suffix: String,
}

impl SafariSettings {
	fn is_default(&self) -> bool {
		*self == Self::default()
	}
}

impl Default for SafariSettings {
	fn default() -> Self {
		Self {
			bundle_id_suffix: "SafariExtension".to_owned(),
		}
	}
}

impl ExtensionConfig {
	/// The pages the configured surfaces declare. The developer tools panel
	/// is two pages: `devtools`, which the developer tools open, and
	/// `devtools_panel`, the panel it adds.
	pub fn pages(&self) -> impl Iterator<Item = PageSurface<'_>> {
		let devtools = self.devtools_page.as_ref();
		[
			self.popup
				.as_ref()
				.map(|page| page.surface("popup", "extension.popup.route")),
			self.side_panel
				.as_ref()
				.map(|page| page.surface("side_panel", "extension.side_panel.route")),
			self.options
				.as_ref()
				.map(|page| page.surface("options", "extension.options.route")),
			self.new_tab
				.as_ref()
				.map(|page| page.surface("new_tab", "extension.new_tab.route")),
			devtools.map(|devtools| {
				PageSurface {
					name: "devtools",
					setting: "extension.devtools_page.route",
					route: devtools.route.as_str(),
					browsers: devtools.browsers.as_deref(),
				}
			}),
			devtools.map(|devtools| {
				PageSurface {
					name: "devtools_panel",
					setting: "extension.devtools_page.panel",
					route: devtools.panel.as_str(),
					browsers: devtools.browsers.as_deref(),
				}
			}),
		]
		.into_iter()
		.flatten()
	}

	/// Whether a surface limited to `only` is packaged for `browser`.
	#[must_use]
	pub fn includes(&self, only: Option<&[Browser]>, browser: Browser) -> bool {
		self.browsers.contains(&browser) && only.is_none_or(|only| only.contains(&browser))
	}

	/// Whether `page` is packaged for `browser`.
	#[must_use]
	pub fn has_page(&self, page: &PageSurface<'_>, browser: Browser) -> bool {
		match page.name {
			"side_panel" => self.has_side_panel(browser),
			"devtools" | "devtools_panel" => self.has_devtools_page(browser),
			_ => self.includes(page.browsers, browser),
		}
	}

	/// Whether the side panel is packaged for `browser`. Safari has no side
	/// panel, so its package leaves it out without being told.
	#[must_use]
	pub fn has_side_panel(&self, browser: Browser) -> bool {
		browser != Browser::Safari
			&& self
				.side_panel
				.as_ref()
				.is_some_and(|page| self.includes(page.browsers.as_deref(), browser))
	}

	/// Whether the developer tools panel is packaged for `browser`. Safari has
	/// no developer tools API, so its package leaves it out without being
	/// told.
	#[must_use]
	pub fn has_devtools_page(&self, browser: Browser) -> bool {
		browser != Browser::Safari
			&& self
				.devtools_page
				.as_ref()
				.is_some_and(|page| self.includes(page.browsers.as_deref(), browser))
	}

	/// The first problem with this configuration, in words.
	#[must_use]
	pub fn problem(&self, package_id: &str) -> Option<String> {
		self.surfaces_problem()
			.or_else(|| self.content_scripts_problem())
			.or_else(|| self.access_problem())
			.or_else(|| self.browser_problem(package_id))
	}

	/// The browsers and the page surfaces.
	fn surfaces_problem(&self) -> Option<String> {
		if self.browsers.is_empty() {
			return Some("extension.browsers must list at least one browser".into());
		}
		for (index, browser) in self.browsers.iter().enumerate() {
			if self.browsers[..index].contains(browser) {
				return Some(format!("extension.browsers lists {browser} more than once"));
			}
		}
		if self.pages().next().is_none()
			&& self.content_scripts.is_empty()
			&& self.background.is_none()
		{
			return Some(
				"[extension] needs at least one surface: popup, side_panel, options, new_tab, \
				 devtools_page, content_scripts, or background"
					.into(),
			);
		}
		if let Some(description) = &self.description
			&& description.chars().count() > 132
		{
			return Some("extension.description must be at most 132 characters".into());
		}
		for page in self.pages() {
			if let Some(problem) = route_problem(page.setting, page.route) {
				return Some(problem);
			}
			if let Some(problem) = self.narrowing_problem(page.table(), page.browsers) {
				return Some(problem);
			}
		}
		if let Some(page) = &self.new_tab
			&& self.browsers.contains(&Browser::Safari)
			&& page
				.browsers
				.as_ref()
				.is_none_or(|only| only.contains(&Browser::Safari))
		{
			return Some(
				"Safari can't replace the new tab page; set extension.new_tab.browsers to the \
				 other browsers"
					.into(),
			);
		}
		None
	}

	/// The content scripts.
	fn content_scripts_problem(&self) -> Option<String> {
		for (index, script) in self.content_scripts.iter().enumerate() {
			let at = format!("extension.content_scripts[{index}]");
			let valid_name = !script.name.is_empty()
				&& script
					.name
					.bytes()
					.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
			if !valid_name {
				return Some(format!(
					"{at}.name `{}` must be lowercase letters, digits, and dashes",
					script.name
				));
			}
			if self.content_scripts[..index]
				.iter()
				.any(|other| other.name == script.name)
			{
				return Some(format!("two content scripts are named `{}`", script.name));
			}
			if let Some(problem) = route_problem(&format!("{at}.route"), &script.route) {
				return Some(problem);
			}
			if script.inject == Inject::Manifest && script.matches.is_empty() {
				return Some(format!(
					"{at} needs `matches`, the pages it runs on, or `inject = \"on_request\"`"
				));
			}
			if let Some(pattern) = script
				.matches
				.iter()
				.find(|pattern| !is_match_pattern(pattern))
			{
				return Some(format!(
					"{at}.matches has `{pattern}`, which isn't a match pattern"
				));
			}
			if let Some(problem) = self.narrowing_problem(&at, script.browsers.as_deref()) {
				return Some(problem);
			}
		}
		None
	}

	/// Permissions, the server, the worker's events, and shortcuts.
	fn access_problem(&self) -> Option<String> {
		for pattern in self
			.host_permissions
			.iter()
			.chain(&self.optional_host_permissions)
		{
			if !is_match_pattern(pattern) {
				return Some(format!(
					"`{pattern}` in the extension's host permissions isn't a match pattern"
				));
			}
		}
		if let Some(server) = &self.server
			&& server_origin(server).is_none()
		{
			return Some(format!(
				"extension.server `{server}` must be an http or https origin, such as https://api.example.com"
			));
		}
		if let Some(background) = &self.background
			&& let Some(event) = background
				.events
				.iter()
				.find(|event| !BACKGROUND_EVENTS.contains(&event.as_str()))
		{
			return Some(format!(
				"extension.background.events has `{event}`; expected one of {}",
				BACKGROUND_EVENTS.join(", ")
			));
		}
		if let Some(name) = self
			.commands
			.keys()
			.find(|name| name.is_empty() || name.starts_with('_'))
		{
			return Some(format!("extension.commands has an invalid name `{name}`"));
		}
		None
	}

	/// Each browser's own settings.
	fn browser_problem(&self, package_id: &str) -> Option<String> {
		if self.browsers.contains(&Browser::Firefox) {
			match &self.firefox {
				None => {
					return Some(
						"Firefox needs an add-on ID: add [extension.firefox] with id = \
						 \"name@example.com\""
							.into(),
					);
				}
				Some(firefox) if !is_gecko_id(&firefox.id) => {
					return Some(format!(
						"extension.firefox.id `{}` must look like an email address, such as \
						 name@example.com",
						firefox.id
					));
				}
				Some(firefox) => {
					if let Some(category) = firefox
						.data_collection
						.iter()
						.find(|category| !FIREFOX_DATA_COLLECTION.contains(&category.as_str()))
					{
						return Some(format!(
							"extension.firefox.data_collection has `{category}`; expected one of \
							 {}",
							FIREFOX_DATA_COLLECTION.join(", ")
						));
					}
					if firefox.data_collection.is_empty()
						|| (firefox.data_collection.len() > 1
							&& firefox
								.data_collection
								.iter()
								.any(|category| category == "none"))
					{
						return Some(
							"extension.firefox.data_collection must be [\"none\"] or the \
							 categories the extension sends"
								.into(),
						);
					}
				}
			}
		}
		if self.browsers.contains(&Browser::Safari) {
			if package_id.is_empty() {
				return Some(
					"Safari's extension lives in the native app, so native.package_id must be set"
						.into(),
				);
			}
			let suffix = &self.safari.bundle_id_suffix;
			let valid = !suffix.is_empty()
				&& suffix
					.bytes()
					.all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
			if !valid {
				return Some(format!(
					"extension.safari.bundle_id_suffix `{suffix}` must be letters, digits, and \
					 dashes"
				));
			}
		}
		None
	}

	/// A surface's own browser list may only narrow the extension's.
	fn narrowing_problem(&self, at: &str, only: Option<&[Browser]>) -> Option<String> {
		let only = only?;
		if only.is_empty() {
			return Some(format!(
				"{at}.browsers can't be empty; leave it out instead"
			));
		}
		only.iter()
			.find(|browser| !self.browsers.contains(browser))
			.map(|browser| format!("{at}.browsers has {browser}, which extension.browsers doesn't"))
	}
}

fn route_problem(at: &str, route: &str) -> Option<String> {
	(!route.starts_with('/'))
		.then(|| format!("{at} `{route}` must be an app route starting with /"))
}

/// The origin of an `http` or `https` URL, such as `https://example.com`.
#[must_use]
pub fn server_origin(server: &str) -> Option<String> {
	let (scheme, rest) = server.split_once("://")?;
	if scheme != "http" && scheme != "https" {
		return None;
	}
	let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
	if host.is_empty() || host.contains('@') || host.contains(' ') {
		return None;
	}
	Some(format!("{scheme}://{host}"))
}

/// Whether `pattern` is a browser extension match pattern: `<all_urls>`, or a
/// scheme, a host, and a path, such as `https://*.example.com/*`.
#[must_use]
pub fn is_match_pattern(pattern: &str) -> bool {
	if pattern == "<all_urls>" {
		return true;
	}
	let Some((scheme, rest)) = pattern.split_once("://") else {
		return false;
	};
	if !matches!(
		scheme,
		"*" | "http" | "https" | "ws" | "wss" | "file" | "ftp"
	) {
		return false;
	}
	let Some((host, _path)) = rest.split_once('/') else {
		return false;
	};
	if scheme == "file" {
		return host.is_empty();
	}
	let host = host.split(':').next().unwrap_or_default();
	match host.strip_prefix("*.") {
		_ if host == "*" => true,
		Some(domain) => !domain.is_empty() && !domain.contains('*'),
		None => !host.is_empty() && !host.contains('*'),
	}
}

/// addons.mozilla.org accepts an email-shaped ID or a GUID in braces.
fn is_gecko_id(id: &str) -> bool {
	let email = id
		.split_once('@')
		.is_some_and(|(name, domain)| !name.is_empty() && !domain.is_empty() && !id.contains(' '));
	let guid = id.starts_with('{') && id.ends_with('}') && id.len() == 38;
	(email || guid) && id.len() <= 80
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn match_patterns() {
		for pattern in [
			"<all_urls>",
			"https://*/*",
			"https://*.example.com/*",
			"http://127.0.0.1/*",
			"*://example.com/path*",
			"file:///*",
		] {
			assert!(is_match_pattern(pattern), "{pattern}");
		}
		for pattern in [
			"example.com",
			"https://ex*ample.com/*",
			"https://example.com",
			"chrome://x/*",
		] {
			assert!(!is_match_pattern(pattern), "{pattern}");
		}
	}

	#[test]
	fn server_origins() {
		assert_eq!(
			server_origin("https://api.example.com/v1").as_deref(),
			Some("https://api.example.com")
		);
		assert_eq!(server_origin("ftp://example.com"), None);
		assert_eq!(server_origin("https://"), None);
	}

	/// An extension whose only surface is a developer tools panel, packaged
	/// for every browser family.
	const DEVTOOLS: &str = r#"
[app]
platforms = ["extension"]
default_platform = "extension"

[native]
package_id = "com.example.inspector"

[extension]
browsers = ["chromium", "firefox", "safari"]

[extension.devtools_page]
route = "/devtools"
panel = "/devtools/panel"

[extension.firefox]
id = "inspector@example.com"
"#;

	/// The validated `[extension]`, or the error and every cause under it.
	fn extension(toml: &str) -> Result<ExtensionConfig, String> {
		fn text(error: &dyn std::error::Error) -> String {
			match error.source() {
				Some(source) => format!("{error}: {}", text(source)),
				None => error.to_string(),
			}
		}
		let config = crate::LeptoskitConfig::from_toml(toml, std::path::Path::new("inspector"))
			.map_err(|error| text(&error))?;
		config.validate().map_err(|error| text(&error))?;
		Ok(config.extension.expect("an [extension] table"))
	}

	#[test]
	fn a_devtools_page_is_two_pages_that_safari_leaves_out() {
		let extension = extension(DEVTOOLS).unwrap();
		let pages: Vec<(&str, &str, &str)> = extension
			.pages()
			.map(|page| (page.name, page.setting, page.route))
			.collect();
		assert_eq!(
			pages,
			[
				("devtools", "extension.devtools_page.route", "/devtools"),
				(
					"devtools_panel",
					"extension.devtools_page.panel",
					"/devtools/panel"
				),
			]
		);
		for page in extension.pages() {
			assert_eq!(page.table(), "extension.devtools_page");
			assert!(extension.has_page(&page, Browser::Chromium));
			assert!(extension.has_page(&page, Browser::Firefox));
			assert!(
				!extension.has_page(&page, Browser::Safari),
				"Safari has no developer tools API"
			);
		}
	}

	#[test]
	fn a_devtools_page_narrows_to_its_own_browsers() {
		let extension = extension(&DEVTOOLS.replace(
			"panel = \"/devtools/panel\"",
			"panel = \"/devtools/panel\"\nbrowsers = [\"firefox\"]",
		))
		.unwrap();
		assert!(!extension.has_devtools_page(Browser::Chromium));
		assert!(extension.has_devtools_page(Browser::Firefox));
	}

	#[test]
	fn configurations_without_a_devtools_page_parse_as_before() {
		let extension = extension(
			"[app]\nplatforms = [\"extension\"]\ndefault_platform = \
			 \"extension\"\n[extension]\nbrowsers = [\"chromium\"]\n[extension.popup]\nroute = \
			 \"/clip\"",
		)
		.unwrap();
		assert_eq!(extension.devtools_page, None);
		assert!(!extension.has_devtools_page(Browser::Chromium));
		let pages: Vec<&str> = extension.pages().map(|page| page.name).collect();
		assert_eq!(pages, ["popup"]);
	}

	#[test]
	fn devtools_page_problems_name_the_setting() {
		let panel = "panel = \"/devtools/panel\"";
		for (from, to, reason) in [
			(
				panel,
				"panel = \"devtools/panel\"",
				"extension.devtools_page.panel `devtools/panel` must be an app route starting \
				 with /",
			),
			(
				"route = \"/devtools\"",
				"route = \"devtools\"",
				"extension.devtools_page.route `devtools` must be an app route starting with /",
			),
			(
				panel,
				"panel = \"/devtools/panel\"\nbrowsers = []",
				"extension.devtools_page.browsers can't be empty",
			),
			(
				panel,
				"panel = \"/devtools/panel\"\ntitle = \"Inspector\"",
				"unknown field `title`",
			),
			("route = \"/devtools\"\n", "", "missing field `route`"),
		] {
			let toml = DEVTOOLS.replace(from, to);
			let error = extension(&toml).unwrap_err();
			assert!(error.contains(reason), "{toml}\n→ {error}");
		}
		let error = extension(
			&DEVTOOLS
				.replace("\"firefox\", \"safari\"", "\"safari\"")
				.replace(
					panel,
					"panel = \"/devtools/panel\"\nbrowsers = [\"firefox\"]",
				),
		)
		.unwrap_err();
		assert!(
			error.contains(
				"extension.devtools_page.browsers has firefox, which extension.browsers doesn't"
			),
			"{error}"
		);
	}
}
