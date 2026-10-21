//! Configuration loading tests.

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_config::ConfigError;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Optimize;
use leptoskit_config::Platform;
use leptoskit_config::RenderMode;
use leptoskit_config::default_package_id;

/// A unique scratch directory removed on drop.
struct Scratch {
	path: PathBuf,
}

impl Scratch {
	fn new(tag: &str) -> Self {
		static COUNTER: AtomicU64 = AtomicU64::new(0);
		let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
		let path = std::env::temp_dir().join(format!(
			"leptoskit_config_test_{tag}_{}_{}",
			std::process::id(),
			unique
		));
		std::fs::create_dir_all(&path).expect("create scratch dir");
		Self { path }
	}
}

impl Drop for Scratch {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.path);
	}
}

#[test]
fn parses_a_full_file() {
	let scratch = Scratch::new("full");
	std::fs::write(
		scratch.path.join("leptoskit.toml"),
		r#"
[app]
name = "counter"
version = "1.2.3"
default_platform = "native"

[serve]
port = 4200

[native]
package_id = "com.example.counter"

[codepush]
enabled = true
server = "https://cp.example.com"
channel = "staging"
public_keys = ["a0d1b6b0b6a5f6cbe4f0e2f6dbb2a1e7b0c3d8f9e1a2b3c4d5e6f708192a3b4c"]

[[plugins]]
name = "size_report"

[[plugins]]
name = "leptos_meta"
leptos = true
"#,
	)
	.expect("write config");
	let config = LeptoskitConfig::load_from(&scratch.path).expect("parse");
	assert_eq!(config.app.name, "counter");
	assert_eq!(config.app.version.to_string(), "1.2.3");
	assert_eq!(config.app.default_platform, Platform::Native);
	assert_eq!(config.serve.port, 4200);
	assert_eq!(config.native.package_id, "com.example.counter");
	assert!(config.codepush.enabled);
	assert_eq!(config.codepush.public_keys.len(), 1);
	assert_eq!(config.plugins.len(), 2);
	assert!(config.plugins[1].leptos);
}

#[test]
fn defaults_fill_from_the_directory() {
	let scratch = Scratch::new("defaults");
	std::fs::write(scratch.path.join("leptoskit.toml"), "").expect("write config");
	let config = LeptoskitConfig::load_from(&scratch.path).expect("parse");
	assert_eq!(
		config.app.name,
		scratch
			.path
			.file_name()
			.and_then(|name| name.to_str())
			.unwrap_or_default()
	);
	assert!(config.native.package_id.starts_with("com.leptoskit."));
	assert_eq!(config.app.default_platform, Platform::Web);
}

#[test]
fn unknown_fields_are_rejected() {
	let scratch = Scratch::new("unknown");
	std::fs::write(
		scratch.path.join("leptoskit.toml"),
		"[app]\nname = \"x\"\nnonexistent = true\n",
	)
	.expect("write config");
	let error = LeptoskitConfig::load_from(&scratch.path).unwrap_err();
	assert!(matches!(error, ConfigError::Parse { .. }), "got {error:?}");
}

#[test]
fn missing_file_reports_read_error() {
	let scratch = Scratch::new("missing");
	let error = LeptoskitConfig::load_from(&scratch.path).unwrap_err();
	assert!(matches!(error, ConfigError::Read { .. }), "got {error:?}");
}

#[test]
fn round_trips_through_toml() {
	let scratch = Scratch::new("roundtrip");
	std::fs::write(
		scratch.path.join("leptoskit.toml"),
		"[app]\nname = \"counter\"\n",
	)
	.expect("write config");
	let config = LeptoskitConfig::load_from(&scratch.path).expect("parse");
	let serialized = config.to_toml_string().expect("serialize");
	let reparsed: LeptoskitConfig = toml::from_str(&serialized).expect("reparse");
	assert_eq!(config, reparsed);
}

fn load(tag: &str, toml: &str) -> Result<LeptoskitConfig, ConfigError> {
	let scratch = Scratch::new(tag);
	std::fs::write(scratch.path.join("leptoskit.toml"), toml).expect("write config");
	LeptoskitConfig::load_from(&scratch.path)
}

#[test]
fn default_package_ids_are_valid_on_ios_and_android() {
	assert_eq!(default_package_id("counter"), "com.leptoskit.counter");
	assert_eq!(default_package_id("my-app"), "com.leptoskit.myapp");
	assert_eq!(default_package_id("my.cool_App"), "com.leptoskit.mycoolapp");
	assert_eq!(default_package_id("1app"), "com.leptoskit.app1app");
	assert_eq!(default_package_id("---"), "com.leptoskit.app");
	assert_eq!(default_package_id(""), "com.leptoskit.app");
	assert_eq!(default_package_id("café"), "com.leptoskit.caf");
}

#[test]
fn derived_package_id_uses_the_app_name() {
	let config = load("derived_id", "[app]\nname = \"my-app\"\n").expect("load");
	assert_eq!(config.native.package_id, "com.leptoskit.myapp");
}

#[test]
fn enabled_codepush_requires_a_server() {
	let error = load("codepush_no_server", "[codepush]\nenabled = true\n").unwrap_err();
	assert!(
		matches!(error, ConfigError::CodePushServerMissing),
		"got {error:?}"
	);
	let error = load(
		"codepush_blank_server",
		"[codepush]\nenabled = true\nserver = \"  \"\n",
	)
	.unwrap_err();
	assert!(
		matches!(error, ConfigError::CodePushServerMissing),
		"got {error:?}"
	);
	assert!(load("codepush_disabled", "[codepush]\nenabled = false\n").is_ok());
}

#[test]
fn enabled_codepush_requires_valid_public_keys() {
	let server = "[codepush]\nenabled = true\nserver = \"https://cp.example.com\"\n";
	let error = load("codepush_no_keys", server).unwrap_err();
	assert!(
		matches!(error, ConfigError::CodePushPublicKeysMissing),
		"got {error:?}"
	);
	for invalid in ["abc", &"g".repeat(64), &"a".repeat(66)] {
		let error = load(
			"codepush_bad_key",
			&format!("{server}public_keys = [\"{invalid}\"]\n"),
		)
		.unwrap_err();
		assert!(
			matches!(&error, ConfigError::InvalidCodePushPublicKey(key) if key == invalid),
			"{invalid:?}: got {error:?}"
		);
	}
	let valid = "a".repeat(64);
	assert!(
		load(
			"codepush_keys",
			&format!("{server}public_keys = [\"{valid}\"]\n")
		)
		.is_ok()
	);
}

#[test]
fn explicit_package_ids_are_validated() {
	for valid in ["com.example.counter", "com.Example.App2", "io.app"] {
		let toml = format!("[native]\npackage_id = \"{valid}\"\n");
		assert!(load("valid_id", &toml).is_ok(), "{valid}");
	}
	for invalid in [
		"counter",
		"com.my_app",
		"com.my-app",
		"com.1app",
		"com..app",
		"com.app.",
	] {
		let toml = format!("[native]\npackage_id = \"{invalid}\"\n");
		let error = load("invalid_id", &toml).unwrap_err();
		assert!(
			matches!(&error, ConfigError::InvalidPackageId { package_id, .. } if package_id == invalid),
			"{invalid}: got {error:?}"
		);
	}
}

#[test]
fn render_mode_defaults_to_csr_and_parses_every_mode() {
	assert_eq!(
		load("render-default", "").expect("parse").app.render,
		RenderMode::Csr
	);
	for (name, mode) in [
		("csr", RenderMode::Csr),
		("ssr", RenderMode::Ssr),
		("islands", RenderMode::Islands),
	] {
		let config = load("render", &format!("[app]\nrender = \"{name}\"\n")).expect("parse");
		assert_eq!(config.app.render, mode);
		assert_eq!(config.app.render.to_string(), name);
	}
	assert!(!RenderMode::Csr.renders_on_server());
	assert!(RenderMode::Ssr.renders_on_server());
	assert!(RenderMode::Islands.renders_on_server());
}

#[test]
fn unknown_render_modes_and_the_old_ssr_flags_are_rejected() {
	for toml in [
		"[app]\nrender = \"spa\"\n",
		"[app]\nssr = true\n",
		"[app]\nhydration = true\n",
	] {
		let error = load("render-bad", toml).unwrap_err();
		assert!(
			matches!(error, ConfigError::Parse { .. }),
			"{toml}: {error:?}"
		);
	}
}

#[test]
fn bundle_settings_default_to_fast_and_parse_smallest() {
	let defaults = load("bundle-default", "").expect("parse").bundle;
	assert_eq!(defaults.optimize, Optimize::Fast);
	assert!(defaults.wasm_opt && defaults.precompress);
	assert_eq!((defaults.cargo_profile, defaults.budget_kib), (None, None));

	let bundle = load(
		"bundle",
		"[bundle]\noptimize = \"smallest\"\ncargo_profile = \"wasm-release\"\nwasm_opt = \
		 false\nprecompress = false\nbudget_kib = 80\n",
	)
	.expect("parse")
	.bundle;
	assert_eq!(bundle.optimize, Optimize::Smallest);
	assert_eq!(bundle.cargo_profile.as_deref(), Some("wasm-release"));
	assert!(!bundle.wasm_opt && !bundle.precompress);
	assert_eq!(bundle.budget_kib, Some(80));

	assert_eq!(
		"smallest".parse::<Optimize>().expect("parse"),
		Optimize::Smallest
	);
	assert_eq!(Optimize::Fast.to_string(), "fast");
	assert!("tiny".parse::<Optimize>().is_err());
}

#[test]
fn bundle_settings_live_in_bundle_not_build() {
	for toml in [
		"[build]\nbudget_kib = 80\n",
		"[bundle]\noptimize = \"tiny\"\n",
	] {
		let error = load("bundle-bad", toml).unwrap_err();
		assert!(
			matches!(error, ConfigError::Parse { .. }),
			"{toml}: {error:?}"
		);
	}
}

#[test]
fn a_key_in_its_old_table_names_where_it_moved() {
	let error = load("moved", "[build]\nwasm_opt = false\n").unwrap_err();
	assert!(matches!(error, ConfigError::MovedKey { .. }), "{error:?}");
	let message = error.to_string();
	assert!(message.contains("build.wasm_opt"), "{message}");
	assert!(message.contains("bundle.wasm_opt"), "{message}");
	assert!(message.contains("[bundle]"), "{message}");
}
