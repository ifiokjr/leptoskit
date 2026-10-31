//! `manifest.json` for each browser family, from `[extension]`.

use std::collections::BTreeSet;

use leptoskit_config::Browser;
use leptoskit_config::ExtensionConfig;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::extension::Inject;
use leptoskit_config::extension::server_origin;
use serde_json::Map;
use serde_json::Value;
use serde_json::json;

/// The policy for the extension's own pages: their scripts and wasm come
/// only from the package.
pub const PAGE_POLICY: &str = "script-src 'self' 'wasm-unsafe-eval'; object-src 'self'";

/// The oldest Firefox that loads module background scripts next to
/// everything else this manifest uses.
pub const FIREFOX_MIN_VERSION: &str = "121.0";

/// What a manifest is built from.
pub struct ManifestInput<'a> {
	pub browser: Browser,
	pub config: &'a LeptoskitConfig,
	pub extension: &'a ExtensionConfig,
	/// Permissions the app's plugins need.
	pub plugin_permissions: &'a [String],
	/// Whether `assets/icon-*.png` files are in the package, by size.
	pub icons: &'a [u32],
	/// A development build: loaded unpacked by `leptoskit dev`.
	pub dev: Option<&'a DevManifest>,
}

/// What a development build adds.
pub struct DevManifest {
	/// The public key that fixes the extension's ID across runs, in base64.
	pub key: String,
	/// The dev server the reload shim polls.
	pub server: String,
}

/// A store's version: one to four dot-separated integers. `1.4.0-beta.2`
/// becomes `1.4.0`; the full version is the second value when it differs.
#[must_use]
pub fn store_version(version: &semver::Version) -> (String, Option<String>) {
	let short = format!("{}.{}.{}", version.major, version.minor, version.patch);
	let full = version.to_string();
	let name = (full != short).then_some(full);
	(short, name)
}

/// The extension's name in stores and the browser.
#[must_use]
pub fn display_name(input: &ManifestInput<'_>) -> String {
	input
		.extension
		.name
		.clone()
		.unwrap_or_else(|| input.config.app.name.clone())
}

/// The manifest for one browser family.
#[must_use]
pub fn manifest(input: &ManifestInput<'_>) -> Value {
	let extension = input.extension;
	let browser = input.browser;
	let name = display_name(input);
	let (version, version_name) = store_version(&input.config.app.version);
	let mut manifest = Map::new();
	manifest.insert("manifest_version".into(), json!(3));
	manifest.insert("name".into(), json!(name));
	manifest.insert("version".into(), json!(version));
	if browser == Browser::Chromium
		&& let Some(version_name) = version_name
	{
		manifest.insert("version_name".into(), json!(version_name));
	}
	if let Some(description) = &extension.description {
		manifest.insert("description".into(), json!(description));
	}

	insert_surfaces(&mut manifest, input, &name);
	let scripts = scripts(input);
	insert_scripts(&mut manifest, input, &scripts);
	insert_access(&mut manifest, input, &scripts);
	Value::Object(manifest)
}

/// The content scripts packaged for this browser.
fn scripts<'a>(input: &ManifestInput<'a>) -> Vec<&'a leptoskit_config::ContentScript> {
	input
		.extension
		.content_scripts
		.iter()
		.filter(|script| {
			input
				.extension
				.includes(script.browsers.as_deref(), input.browser)
		})
		.collect()
}

/// The toolbar button, icons, and the page surfaces.
fn insert_surfaces(manifest: &mut Map<String, Value>, input: &ManifestInput<'_>, name: &str) {
	let extension = input.extension;
	let browser = input.browser;
	let mut action = Map::new();
	action.insert("default_title".into(), json!(name));
	if let Some(popup) = &extension.popup
		&& extension.includes(popup.browsers.as_deref(), browser)
	{
		action.insert("default_popup".into(), json!("popup.html"));
	}
	if !input.icons.is_empty() {
		action.insert("default_icon".into(), icons(input.icons));
		manifest.insert("icons".into(), icons(input.icons));
	}
	manifest.insert("action".into(), Value::Object(action));

	if let Some(options) = &extension.options
		&& extension.includes(options.browsers.as_deref(), browser)
	{
		manifest.insert(
			"options_ui".into(),
			json!({ "page": "options.html", "open_in_tab": true }),
		);
	}
	if let Some(new_tab) = &extension.new_tab
		&& extension.includes(new_tab.browsers.as_deref(), browser)
	{
		manifest.insert(
			"chrome_url_overrides".into(),
			json!({ "newtab": "new_tab.html" }),
		);
	}
	if extension.has_side_panel(browser) {
		match browser {
			Browser::Chromium => {
				manifest.insert(
					"side_panel".into(),
					json!({ "default_path": "side_panel.html" }),
				);
			}
			Browser::Firefox => {
				manifest.insert(
					"sidebar_action".into(),
					json!({ "default_panel": "side_panel.html", "default_title": name }),
				);
			}
			Browser::Safari => {}
		}
	}
}

/// The background worker, and the content scripts with what their frames
/// load.
fn insert_scripts(
	manifest: &mut Map<String, Value>,
	input: &ManifestInput<'_>,
	scripts: &[&leptoskit_config::ContentScript],
) {
	let extension = input.extension;
	let browser = input.browser;
	if extension.background.is_some() || input.dev.is_some() {
		manifest.insert(
			"background".into(),
			match browser {
				// Firefox runs background scripts as event pages, not service
				// workers; the module loads the same way in both.
				Browser::Firefox => json!({ "scripts": ["background.js"], "type": "module" }),
				Browser::Chromium | Browser::Safari => {
					json!({ "service_worker": "background.js", "type": "module" })
				}
			},
		);
	}

	let declared: Vec<Value> = scripts
		.iter()
		.filter(|script| script.inject == Inject::Manifest)
		.map(|script| {
			json!({
				"matches": script.matches,
				"js": [format!("content-{}.js", script.name)],
				"run_at": script.run_at.manifest(),
			})
		})
		.collect();
	if !declared.is_empty() {
		manifest.insert("content_scripts".into(), Value::Array(declared));
	}
	if !scripts.is_empty() {
		// The overlay frame each script shows, and the app it runs. A script
		// injected on request or registered later can reach any page.
		let mut matches: BTreeSet<String> = BTreeSet::new();
		for script in scripts {
			if script.inject == Inject::Manifest {
				matches.extend(script.matches.iter().cloned());
			} else {
				matches.insert("<all_urls>".into());
			}
		}
		let mut resources = json!({
			"resources": ["overlay.html", "overlay.js", "pkg/*", "assets/*"],
			"matches": matches.into_iter().collect::<Vec<_>>(),
		});
		// Chromium can change the URL every session, so pages can't detect the
		// extension by fetching a fixed address.
		if browser == Browser::Chromium {
			resources["use_dynamic_url"] = json!(true);
		}
		manifest.insert("web_accessible_resources".into(), json!([resources]));
	}
}

/// Permissions, host access, shortcuts, the pages' policy, and each
/// browser's own settings.
fn insert_access(
	manifest: &mut Map<String, Value>,
	input: &ManifestInput<'_>,
	scripts: &[&leptoskit_config::ContentScript],
) {
	let extension = input.extension;
	let browser = input.browser;
	let permissions = permissions(input, scripts);
	if !permissions.is_empty() {
		manifest.insert("permissions".into(), json!(permissions));
	}
	let mut hosts: BTreeSet<String> = extension.host_permissions.iter().cloned().collect();
	if let Some(origin) = extension.server.as_deref().and_then(server_origin) {
		hosts.insert(origin_pattern(&origin));
	}
	for script in scripts {
		if script.inject == Inject::Manifest {
			hosts.extend(script.matches.iter().cloned());
		}
	}
	if let Some(dev) = input.dev
		&& let Some(origin) = server_origin(&dev.server)
	{
		hosts.insert(origin_pattern(&origin));
	}
	if !hosts.is_empty() {
		manifest.insert("host_permissions".into(), json!(hosts));
	}
	if !extension.optional_host_permissions.is_empty() {
		let optional: BTreeSet<&String> = extension.optional_host_permissions.iter().collect();
		manifest.insert("optional_host_permissions".into(), json!(optional));
	}

	if !extension.commands.is_empty() {
		let commands: Map<String, Value> = extension
			.commands
			.iter()
			.map(|(name, command)| {
				let mut value = json!({ "description": command.description });
				if let Some(key) = &command.suggested_key {
					value["suggested_key"] = json!({ "default": key });
				}
				(name.clone(), value)
			})
			.collect();
		manifest.insert("commands".into(), Value::Object(commands));
	}

	manifest.insert(
		"content_security_policy".into(),
		json!({ "extension_pages": PAGE_POLICY }),
	);
	if browser == Browser::Firefox
		&& let Some(firefox) = &extension.firefox
	{
		manifest.insert(
			"browser_specific_settings".into(),
			json!({
				"gecko": {
					"id": firefox.id,
					"strict_min_version": firefox
						.strict_min_version
						.as_deref()
						.unwrap_or(FIREFOX_MIN_VERSION),
					"data_collection_permissions": { "required": firefox.data_collection },
				}
			}),
		);
	}
	if let Some(dev) = input.dev
		&& browser == Browser::Chromium
	{
		manifest.insert("key".into(), json!(dev.key));
	}
}

/// The match pattern for every page on an origin. Match patterns can't name
/// a port, so a pattern covers every port on the host.
fn origin_pattern(origin: &str) -> String {
	let (scheme, host) = origin.split_once("://").unwrap_or(("https", origin));
	let host = match host.rsplit_once(':') {
		Some((name, port)) if port.bytes().all(|byte| byte.is_ascii_digit()) => name,
		_ => host,
	};
	format!("{scheme}://{host}/*")
}

fn icons(sizes: &[u32]) -> Value {
	Value::Object(
		sizes
			.iter()
			.map(|size| (size.to_string(), json!(format!("assets/icon-{size}.png"))))
			.collect(),
	)
}

/// Every permission the package asks for: the configured ones, the
/// plugins', and those its own surfaces need, sorted.
fn permissions(
	input: &ManifestInput<'_>,
	scripts: &[&leptoskit_config::ContentScript],
) -> Vec<String> {
	let extension = input.extension;
	let mut permissions: BTreeSet<String> = extension.permissions.iter().cloned().collect();
	permissions.extend(input.plugin_permissions.iter().cloned());
	if extension.has_side_panel(input.browser) && input.browser == Browser::Chromium {
		permissions.insert("sidePanel".into());
	}
	if scripts
		.iter()
		.any(|script| script.inject != Inject::Manifest)
	{
		permissions.insert("scripting".into());
	}
	// The runtime keeps the session token and its own state in extension
	// storage, shared by every surface and the worker.
	permissions.insert("storage".into());
	if let Some(background) = &extension.background {
		for event in &background.events {
			match event.as_str() {
				"alarms" => permissions.insert("alarms".into()),
				"context_menus" => permissions.insert("contextMenus".into()),
				_ => false,
			};
		}
	}
	permissions.into_iter().collect()
}
