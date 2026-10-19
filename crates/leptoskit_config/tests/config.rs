//! Configuration loading tests.

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_config::ConfigError;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;

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
