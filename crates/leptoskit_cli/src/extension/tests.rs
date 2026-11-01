//! The manifests every browser family gets, and the route check.

use leptoskit_config::Browser;
use leptoskit_config::LeptoskitConfig;

use super::declared_permissions;
use super::manifest::DevManifest;
use super::manifest::ManifestInput;
use super::manifest::manifest;
use super::manifest::store_version;
use super::pages::Module;
use super::plugin_permissions;
use super::route_matches;
use super::write_package;
use super::zip;

/// Commonplace's configuration: every surface, each injection mode, a
/// command, and all three browsers.
const COMMONPLACE: &str = r#"
[app]
name = "commonplace"
version = "1.4.0-beta.2"
platforms = ["web", "extension"]

[native]
package_id = "com.leptoskit.commonplace"

[extension]
name = "Commonplace"
description = "Save quotes from the web into your research projects."
browsers = ["chromium", "firefox", "safari"]
server = "https://commonplace.example.com/api"
permissions = ["activeTab"]
optional_host_permissions = ["https://*/*"]

[extension.popup]
route = "/clip"

[extension.side_panel]
route = "/panel"

[extension.options]
route = "/settings"

[extension.new_tab]
route = "/reading"
browsers = ["chromium", "firefox"]

[[extension.content_scripts]]
name = "clipper"
route = "/overlay/clipper"
inject = "on_request"

[[extension.content_scripts]]
name = "highlights"
route = "/overlay/highlights"
inject = "registered"

[[extension.content_scripts]]
name = "docs"
route = "/overlay/docs"
matches = ["https://docs.example.com/*"]
run_at = "document_end"

[extension.background]
events = ["runtime.message", "runtime.installed", "alarms", "context_menus", "commands"]

[extension.commands.clip]
suggested_key = "Alt+Shift+C"
description = "Clip the selection"

[extension.firefox]
id = "commonplace@leptoskit.dev"
data_collection = ["websiteContent", "personallyIdentifyingInfo"]
"#;

/// A developer tools panel, shaped like leptoskit's own: its only surface,
/// for every family, reaching a server on loopback.
const DEVTOOLS: &str = r#"
[app]
name = "inspector"
version = "0.3.0"
platforms = ["extension"]
default_platform = "extension"

[native]
package_id = "com.example.inspector"

[extension]
name = "Inspector"
browsers = ["chromium", "firefox", "safari"]
host_permissions = ["http://127.0.0.1/*"]

[extension.devtools_page]
route = "/devtools"
panel = "/devtools/panel"

[extension.firefox]
id = "inspector@example.com"
"#;

fn config(toml: &str) -> LeptoskitConfig {
	let config = LeptoskitConfig::from_toml(toml, std::path::Path::new("commonplace")).unwrap();
	config.validate().unwrap();
	config
}

#[test]
fn every_browser_gets_its_own_manifest() {
	let config = config(COMMONPLACE);
	let extension = config.extension.as_ref().unwrap();

	for browser in Browser::ALL {
		let manifest = manifest(&ManifestInput {
			browser,
			config: &config,
			extension,
			plugin_permissions: &["notifications".to_owned()],
			icons: &[16, 128],
			dev: None,
		});
		insta::assert_json_snapshot!(format!("manifest_{browser}"), manifest);
	}
}

#[test]
fn the_developer_tools_page_is_declared_for_chromium_and_firefox() {
	let config = config(DEVTOOLS);
	let extension = config.extension.as_ref().unwrap();

	for browser in Browser::ALL {
		let manifest = manifest(&ManifestInput {
			browser,
			config: &config,
			extension,
			plugin_permissions: &[],
			icons: &[],
			dev: None,
		});
		// Safari has no developer tools API.
		let page = (browser != Browser::Safari).then_some("devtools.html");
		assert_eq!(manifest["devtools_page"].as_str(), page, "{browser}");

		if browser != Browser::Safari {
			insta::assert_json_snapshot!(format!("manifest_devtools_{browser}"), manifest);
		}
	}
}

#[test]
fn packages_carry_the_developer_tools_pages_except_safari_s() {
	let config = config(DEVTOOLS);
	let extension = config.extension.as_ref().unwrap();
	let build = tempfile::tempdir().unwrap();
	let pkg = build.path().join("pkg");
	std::fs::create_dir_all(&pkg).unwrap();
	std::fs::write(pkg.join("inspector_bg.wasm"), b"\0asm").unwrap();
	std::fs::write(pkg.join("inspector.js"), "export default () => {};").unwrap();
	let module = Module { stem: "inspector" };

	let pages = [
		"devtools.html",
		"devtools.js",
		"devtools_panel.html",
		"devtools_panel.js",
	];

	for browser in Browser::ALL {
		let dir = build.path().join(browser.name());
		let input = ManifestInput {
			browser,
			config: &config,
			extension,
			plugin_permissions: &[],
			icons: &[],
			dev: None,
		};
		write_package(
			&dir,
			&input,
			&module,
			&pkg,
			&build.path().join("assets"),
			None,
		)
		.unwrap();
		let files = zip::files_under(&dir).unwrap();
		let archive = zip::archive(&files).unwrap();

		for page in pages {
			let packaged = files.iter().any(|(name, _)| name == page);
			let zipped = archive
				.windows(page.len())
				.any(|window| window == page.as_bytes());
			let expected = browser != Browser::Safari;
			assert_eq!(packaged, expected, "{browser}: {page}");
			assert_eq!(zipped, expected, "{browser}'s zip: {page}");
		}
	}

	let chromium = build.path().join("chromium");
	let read = |name: &str| std::fs::read_to_string(chromium.join(name)).unwrap();
	assert!(
		read("devtools.js").contains("leptoskit_extension_start(\"devtools\", \"/devtools\");"),
		"{}",
		read("devtools.js")
	);
	assert!(
		read("devtools_panel.js")
			.contains("leptoskit_extension_start(\"devtools_panel\", \"/devtools/panel\");")
	);
	for (page, script) in [
		("devtools.html", "devtools.js"),
		("devtools_panel.html", "devtools_panel.js"),
	] {
		assert!(
			read(page).contains(&format!(
				"<script type=\"module\" src=\"{script}\"></script>"
			)),
			"{page}"
		);
	}
}

#[test]
fn a_dev_build_has_a_fixed_key_and_reaches_the_dev_server() {
	let config = config(
		r#"
[app]
platforms = ["extension"]
default_platform = "extension"
[extension]
browsers = ["chromium"]
[extension.popup]
route = "/"
"#,
	);
	let dev = DevManifest {
		key: Some("MIIBIjANBgkq".into()),
		server: "http://127.0.0.1:3000".into(),
	};
	let manifest = manifest(&ManifestInput {
		browser: Browser::Chromium,
		config: &config,
		extension: config.extension.as_ref().unwrap(),
		plugin_permissions: &[],
		icons: &[],
		dev: Some(&dev),
	});
	assert_eq!(manifest["key"], "MIIBIjANBgkq");
	assert_eq!(
		manifest["host_permissions"],
		serde_json::json!(["http://127.0.0.1/*"])
	);
	assert_eq!(manifest["background"]["service_worker"], "background.js");
}

#[test]
fn plugins_in_the_client_s_graph_add_their_permissions() {
	let metadata = serde_json::json!({
		"packages": [
			{ "id": "app", "name": "app", "metadata": null },
			{ "id": "notify", "name": "leptoskit_plugin_notifications",
			  "metadata": { "leptoskit": { "runtime": { "extension-permissions": ["notifications"] } } } },
			{ "id": "clip", "name": "leptoskit_plugin_clipboard",
			  "metadata": { "leptoskit": { "runtime": { "extension-permissions": ["clipboardWrite"] } } } },
		],
		"resolve": { "nodes": [
			{ "id": "app", "deps": [{ "pkg": "notify" }] },
			{ "id": "notify", "deps": [] },
			{ "id": "clip", "deps": [] },
		]},
	});
	// The clipboard plugin is in the workspace, but not the app's graph.
	assert_eq!(declared_permissions(&metadata, "app"), ["notifications"]);
}

#[test]
fn repeated_and_unknown_packages_are_skipped() {
	let metadata = serde_json::json!({
		"packages": [
			{ "id": "app", "name": "app", "metadata": null },
			{ "id": "notify", "name": "notify",
			  "metadata": { "leptoskit": { "runtime": { "extension-permissions": ["notifications"] } } } },
		],
		"resolve": { "nodes": [
			{ "id": "app", "deps": [{ "pkg": "notify" }, { "pkg": "notify" }, { "pkg": "gone" }] },
			{ "id": "notify", "deps": [{ "pkg": "app" }] },
		]},
	});
	// The duplicate, the cycle back to the app, and the package that isn't in
	// the graph are all walked once or skipped.
	assert_eq!(declared_permissions(&metadata, "app"), ["notifications"]);
}

#[test]
fn plugin_permissions_reads_the_workspace_metadata() {
	let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
	let permissions = plugin_permissions(root, "leptoskit_plugin_notifications")
		.expect("the workspace's cargo metadata");
	assert!(permissions.contains(&"notifications".to_owned()));
}

#[test]
fn store_versions_drop_prerelease_parts() {
	let version = semver::Version::parse("1.4.0-beta.2").unwrap();
	assert_eq!(
		store_version(&version),
		("1.4.0".into(), Some("1.4.0-beta.2".into()))
	);
	let version = semver::Version::parse("2.0.1").unwrap();
	assert_eq!(store_version(&version), ("2.0.1".into(), None));
}

#[test]
fn configured_routes_match_the_app_s_routes() {
	assert!(route_matches("/", "/"));
	assert!(route_matches("/clip", "/clip"));
	assert!(route_matches("/projects/:project", "/projects/42"));
	assert!(!route_matches("/projects/:project", "/projects"));
	assert!(route_matches("/help/*topic", "/help/sync/ios"));
	assert!(route_matches("/*path?", "/anything/at/all"));
	assert!(!route_matches("/clip", "/clips"));
}

#[test]
fn invalid_extensions_are_refused_with_the_reason() {
	let cases = [
		(
			"[app]\nplatforms = [\"extension\"]\ndefault_platform = \"extension\"",
			"add an [extension] table",
		),
		(
			"[app]\nplatforms = [\"extension\"]\ndefault_platform = \"extension\"\n[extension]\nbrowsers = [\"chromium\"]",
			"needs at least one surface",
		),
		(
			"[app]\nplatforms = [\"extension\"]\ndefault_platform = \"extension\"\n[extension]\n[extension.popup]\nroute = \"/\"",
			"Firefox needs an add-on ID",
		),
		(
			"[app]\nplatforms = [\"extension\"]\ndefault_platform = \"extension\"\n[extension]\nbrowsers = [\"chromium\"]\n[extension.popup]\nroute = \"clip\"",
			"must be an app route starting with /",
		),
		(
			"[app]\nplatforms = [\"extension\"]\ndefault_platform = \"extension\"\n[extension]\nbrowsers = [\"chromium\", \"safari\"]\n[extension.new_tab]\nroute = \"/\"",
			"Safari can't replace the new tab page",
		),
		(
			"[app]\nplatforms = [\"extension\"]\ndefault_platform = \"extension\"\n[extension]\nbrowsers = [\"chromium\"]\n[[extension.content_scripts]]\nname = \"x\"\nroute = \"/\"",
			"needs `matches`",
		),
		(
			"[app]\nplatforms = [\"extension\"]\ndefault_platform = \"extension\"\n[codepush]\nenabled = true\nserver = \"https://x\"\npublic_keys = [\"0000000000000000000000000000000000000000000000000000000000000000\"]\n[extension]\nbrowsers = [\"chromium\"]\n[extension.popup]\nroute = \"/\"",
			"code push",
		),
	];

	for (toml, reason) in cases {
		let error = LeptoskitConfig::from_toml(toml, std::path::Path::new("app"))
			.and_then(|config| config.validate())
			.unwrap_err()
			.to_string();
		assert!(error.contains(reason), "{toml}\n→ {error}");
	}
}
