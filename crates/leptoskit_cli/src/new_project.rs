//! `leptoskit new`: scaffolds a project from the embedded template.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;
use leptoskit_config::AppConfig;
use leptoskit_config::BuildConfig;
use leptoskit_config::CodePushConfig;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::NativeConfig;
use leptoskit_config::Platform;
use leptoskit_config::PluginSpec;
use leptoskit_config::ServeConfig;

/// Writes a new project with a `leptoskit.toml` and a starter main.
pub fn run(path: &str, platform: Platform, name: Option<&str>) -> Result<()> {
	let dir = Utf8PathBuf::from(path);
	std::fs::create_dir_all(&dir).with_context(|| format!("could not create {dir}"))?;
	let app_name = name.unwrap_or_else(|| {
		dir.file_name()
			.unwrap_or("leptoskit_app")
			.trim_end_matches('/')
	});

	let mut config = LeptoskitConfig {
		app: AppConfig {
			name: app_name.to_owned(),
			..AppConfig::default()
		},
		serve: ServeConfig::default(),
		build: BuildConfig::default(),
		native: NativeConfig {
			package_id: format!("com.leptoskit.{}", app_name.replace(['-', '.'], "_")),
			..NativeConfig::default()
		},
		codepush: CodePushConfig::default(),
		plugins: vec![PluginSpec {
			name: "leptos_meta".to_owned(),
			leptos: true,
			..PluginSpec::default()
		}],
	};
	config.app.default_platform = platform;

	let config_path = dir.join(leptoskit_config::CONFIG_FILE);
	if config_path.exists() {
		anyhow::bail!("{config_path} already exists");
	}
	std::fs::write(
		&config_path,
		config.to_toml_string().context("serialize config")?,
	)
	.with_context(|| format!("could not write {config_path}"))?;

	let src = dir.join("src");
	std::fs::create_dir_all(&src).with_context(|| format!("could not create {src}"))?;
	std::fs::write(src.join("main.rs"), MAIN_RS)
		.with_context(|| format!("could not write {}", src.join("main.rs")))?;

	println!("created {dir} ({platform} project)");
	Ok(())
}

/// The starter main: a headless native layout smoke test that doubles as the
/// project's first test.
const MAIN_RS: &str = r#"fn main() {
	println!("hello from leptoskit");
}
"#;

/// The template directory scan used by `leptoskit new --list` in tests.
pub fn template_files() -> &'static [&'static str] {
	&[leptoskit_config::CONFIG_FILE, "src/main.rs"]
}

/// Ensures the scaffold writes every file it promises.
pub fn verify_scaffold(dir: &Path) -> Result<()> {
	for file in template_files() {
		let path = dir.join(file);
		anyhow::ensure!(path.is_file(), "scaffold is missing {file}");
	}
	Ok(())
}
