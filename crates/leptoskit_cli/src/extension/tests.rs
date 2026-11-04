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

#[test]
fn firefox_dev_manifest_uses_an_event_page_and_loopback_permissions() {
	let config = config(COMMONPLACE);
	let dev = DevManifest {
		key: None,
		server: "http://127.0.0.1:7341".into(),
	};
	let manifest = manifest(&ManifestInput {
		browser: Browser::Firefox,
		config: &config,
		extension: config.extension.as_ref().unwrap(),
		plugin_permissions: &[],
		icons: &[],
		dev: Some(&dev),
	});
	assert_eq!(
		manifest["background"],
		serde_json::json!({"scripts":["background.js"], "type":"module"})
	);
	assert!(
		manifest["host_permissions"]
			.as_array()
			.unwrap()
			.iter()
			.any(|p| p == "http://127.0.0.1/*")
	);
	assert!(manifest.get("key").is_none());
	assert_eq!(
		manifest["browser_specific_settings"]["gecko"]["id"],
		"commonplace@leptoskit.dev"
	);
}

#[test]
fn extension_browser_flags_are_parsed_and_invalid_browsers_are_rejected() {
	use clap::Parser as _;

	use super::browser::ExtensionBrowser;
	use crate::cli::Cli;
	use crate::cli::Command;
	let dev = Cli::try_parse_from([
		"lk",
		"dev",
		"--platform",
		"extension",
		"--browser",
		"firefox",
	])
	.unwrap();
	assert!(matches!(
		dev.command,
		Command::Dev {
			browser: ExtensionBrowser::Firefox,
			..
		}
	));
	let test = Cli::try_parse_from(["lk", "test", "extension", "--browser", "firefox"]).unwrap();
	assert!(matches!(
		test.command,
		Command::Test {
			command: crate::testing::TestCommand::Extension {
				browser: ExtensionBrowser::Firefox
			}
		}
	));
	assert!(Cli::try_parse_from(["lk", "test", "extension", "--browser", "safari"]).is_err());
}

#[test]
fn firefox_source_archives_keep_rebuild_inputs_and_exclude_even_tracked_secrets() {
	for tracked in [false, true] {
		let root = tempfile::tempdir().unwrap();
		let write = |name: &str, contents: &str| {
			let path = root.path().join(name);
			std::fs::create_dir_all(path.parent().unwrap()).unwrap();
			std::fs::write(path, contents).unwrap();
		};
		write("src/lib.rs", "pub fn source_marker() {}\n");
		write(
			"Cargo.toml",
			"[package]\nname = 'fixture'\nversion = '0.1.0'\n",
		);
		write(
			"monosecret.toml",
			"[secrets.DATABASE_URL]\nprovider = 'env'\n",
		);
		write("BUILD.md", "obsolete instructions");
		write(".env", "SECRET_ENV_MARKER");
		write("keys/signing.key", "SECRET_KEY_MARKER");
		write("keys/apple.p8", "SECRET_APPLE_MARKER");
		write("keys/certificate.pem", "SECRET_PEM_MARKER");
		write("android/upload.jks", "SECRET_KEYSTORE_MARKER");
		let outside = tempfile::tempdir().unwrap();
		std::fs::write(outside.path().join("credentials"), "SECRET_LINKED_MARKER").unwrap();
		#[cfg(unix)]
		std::os::unix::fs::symlink(
			outside.path().join("credentials"),
			root.path().join("notes.md"),
		)
		.unwrap();
		#[cfg(not(unix))]
		write("notes.md", "notes");
		write("target/cache", "BUILD_OUTPUT_MARKER");
		write(".gitignore", "/target/\n");
		if tracked {
			assert!(
				crate::git::command(root.path())
					.arg("init")
					.output()
					.unwrap()
					.status
					.success()
			);
			assert!(
				crate::git::command(root.path())
					.args([
						"add",
						"--force",
						"src",
						"Cargo.toml",
						"monosecret.toml",
						"BUILD.md",
						".env",
						"keys",
						"android",
						"notes.md"
					])
					.output()
					.unwrap()
					.status
					.success()
			);
		}
		let archive = super::source_archive(root.path(), &config(COMMONPLACE), tracked).unwrap();
		let entries = unzip_for_test(&archive);
		let contents = entries
			.iter()
			.map(|(name, contents)| format!("{name}\n{}", String::from_utf8_lossy(contents)))
			.collect::<Vec<_>>()
			.join("\n");
		for kept in [
			"source_marker",
			"Cargo.toml",
			"monosecret.toml",
			"Building commonplace for Firefox",
			"rustup target add wasm32-unknown-unknown",
		] {
			assert!(contents.contains(kept), "missing {kept}");
		}
		for excluded in [
			"SECRET_ENV_MARKER",
			"SECRET_KEY_MARKER",
			"SECRET_APPLE_MARKER",
			"SECRET_PEM_MARKER",
			"SECRET_KEYSTORE_MARKER",
			"SECRET_LINKED_MARKER",
			"BUILD_OUTPUT_MARKER",
			"obsolete instructions",
		] {
			assert!(
				!contents.contains(excluded),
				"source archive leaked {excluded}"
			);
		}
		assert_eq!(contents.contains("extension --release"), tracked);
	}
}

#[test]
fn extension_routes_are_checked_before_packaging_every_surface() {
	let root = tempfile::tempdir().unwrap();
	let routes = root.path().join("pages");
	for page in [
		"clip",
		"panel",
		"settings",
		"reading",
		"overlay/clipper",
		"overlay/highlights",
		"overlay/docs",
	] {
		let path = routes.join(format!("{page}.rs"));
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, "pub fn page() {}\n").unwrap();
	}
	let mut config = config(COMMONPLACE);
	config.router = Some(leptoskit_config::RouterConfig {
		dir: "pages".into(),
	});
	let mut extension = config.extension.clone().unwrap();
	extension.popup.as_mut().unwrap().route = "/clip?tab=saved#first".into();
	super::check_routes(root.path(), &config, &extension).unwrap();
	extension.side_panel.as_mut().unwrap().route = "/missing".into();
	let error = super::check_routes(root.path(), &config, &extension)
		.unwrap_err()
		.to_string();
	assert!(
		error.contains("extension.side_panel")
			&& error.contains("/missing")
			&& error.contains("pages"),
		"{error}"
	);
	// An app without a file router owns routing itself.
	config.router = None;
	super::check_routes(root.path(), &config, &extension).unwrap();
}

/// Reads the ZIP local records so assertions inspect decompressed source,
/// rather than depending on whether a particular entry compresses well.
fn unzip_for_test(archive: &[u8]) -> std::collections::BTreeMap<String, Vec<u8>> {
	use std::io::Read as _;
	let mut entries = std::collections::BTreeMap::new();
	let mut offset = 0;
	while archive.get(offset..offset + 4) == Some(b"PK\x03\x04") {
		let u16_at = |at| usize::from(u16::from_le_bytes(archive[at..at + 2].try_into().unwrap()));
		let method = u16_at(offset + 8);
		let compressed = usize::try_from(u32::from_le_bytes(
			archive[offset + 18..offset + 22].try_into().unwrap(),
		))
		.unwrap();
		let name_len = u16_at(offset + 26);
		let extra_len = u16_at(offset + 28);
		let name =
			String::from_utf8(archive[offset + 30..offset + 30 + name_len].to_vec()).unwrap();
		let start = offset + 30 + name_len + extra_len;
		let data = &archive[start..start + compressed];
		let contents = match method {
			0 => data.to_vec(),
			8 => {
				let mut contents = Vec::new();
				flate2::read::DeflateDecoder::new(data)
					.read_to_end(&mut contents)
					.unwrap();
				contents
			}
			other => panic!("unsupported ZIP method {other}"),
		};
		assert!(entries.insert(name, contents).is_none());
		offset = start + compressed;
	}
	assert!(!entries.is_empty());
	entries
}
