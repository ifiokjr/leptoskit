//! Experimental Wear OS and watchOS host scaffolding.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::NativeTarget;

use super::Target;

/// Validates that a watch starter's package id is set.
pub fn validate(config: &LeptoskitConfig) -> Result<()> {
	anyhow::ensure!(
		!config.native.package_id.is_empty(),
		"native.package_id is required for Wear OS and watchOS starters"
	);
	Ok(())
}

/// Writes the experimental host stubs for `target` under
/// `hosts/{wearos|watchos}`.
pub fn scaffold(root: &Path, config: &LeptoskitConfig, target: NativeTarget) -> Result<()> {
	validate(config)?;
	match target {
		NativeTarget::Wearos => scaffold_wearos(root, config),
		NativeTarget::Watchos => scaffold_watchos(root, config),
		other => anyhow::bail!("{other} is not a watch target"),
	}
}

fn scaffold_wearos(root: &Path, config: &LeptoskitConfig) -> Result<()> {
	let host = root.join("hosts/wearos");
	write(
		&host.join("readme.md"),
		include_str!("templates/wearos/readme.md")
			.replace("__PACKAGE_ID__", &config.native.package_id)
			.replace("__APP_NAME__", &config.app.name),
	)?;
	write(
		&host.join("android/WatchActivity.kt"),
		include_str!("templates/wearos/WatchActivity.kt"),
	)?;
	write(
		&host.join("android/TileService.kt"),
		include_str!("templates/wearos/TileService.kt"),
	)?;
	write(
		&host.join("android/ComplicationDataSource.kt"),
		include_str!("templates/wearos/ComplicationDataSource.kt"),
	)?;
	write(
		&host.join("guest.rs"),
		include_str!("templates/wearos/guest.rs"),
	)?;
	Ok(())
}

fn scaffold_watchos(root: &Path, config: &LeptoskitConfig) -> Result<()> {
	let host = root.join("hosts/watchos");
	write(
		&host.join("readme.md"),
		include_str!("templates/watchos/readme.md")
			.replace("__PACKAGE_ID__", &config.native.package_id)
			.replace("__APP_NAME__", &config.app.name),
	)?;
	write(
		&host.join("swift/WatchApp.swift"),
		include_str!("templates/watchos/WatchApp.swift"),
	)?;
	write(
		&host.join("swift/GlanceWidget.swift"),
		include_str!("templates/watchos/GlanceWidget.swift"),
	)?;
	write(
		&host.join("swift/ComplicationController.swift"),
		include_str!("templates/watchos/ComplicationController.swift"),
	)?;
	write(
		&host.join("guest.rs"),
		include_str!("templates/watchos/guest.rs"),
	)?;
	Ok(())
}

fn write(path: &Path, content: impl AsRef<[u8]>) -> Result<()> {
	let parent = path
		.parent()
		.with_context(|| format!("could not create parent for {}", path.display()))?;
	std::fs::create_dir_all(parent)
		.with_context(|| format!("could not create {}", parent.display()))?;
	std::fs::write(path, content).with_context(|| format!("could not write {}", path.display()))
}

/// Explains that packaging is still experimental.
pub fn build(target: Target) -> Result<()> {
	let name = match target {
		Target::Wearos => "Wear OS",
		Target::Watchos => "watchOS",
		other => anyhow::bail!("{other:?} is not a watch target"),
	};
	anyhow::bail!(
		"{name} support is experimental: the host stubs and watch API are ready, but packaging \
		 and device install are not implemented yet. See the {name} platform page in the docs."
	)
}

#[cfg(test)]
mod tests {
	use leptoskit_config::AppConfig;
	use leptoskit_config::NativeConfig;

	use super::*;

	#[test]
	fn scaffolds_wearos_host_files() {
		let root = std::env::temp_dir().join(format!("leptoskit-wearos-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		let config = LeptoskitConfig {
			app: AppConfig {
				name: "steps".into(),
				..AppConfig::default()
			},
			native: NativeConfig {
				package_id: "dev.leptoskit.steps".into(),
				targets: vec![NativeTarget::Wearos],
				..NativeConfig::default()
			},
			..LeptoskitConfig::default()
		};
		scaffold(&root, &config, NativeTarget::Wearos).unwrap();
		assert!(root.join("hosts/wearos/android/TileService.kt").is_file());
		assert!(root.join("hosts/wearos/guest.rs").is_file());
		let _ = std::fs::remove_dir_all(&root);
	}

	#[test]
	fn scaffolds_watchos_host_files() {
		let root = std::env::temp_dir().join(format!("leptoskit-watchos-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		let config = LeptoskitConfig {
			app: AppConfig {
				name: "steps".into(),
				..AppConfig::default()
			},
			native: NativeConfig {
				package_id: "dev.leptoskit.steps".into(),
				targets: vec![NativeTarget::Watchos],
				..NativeConfig::default()
			},
			..LeptoskitConfig::default()
		};
		scaffold(&root, &config, NativeTarget::Watchos).unwrap();
		assert!(root.join("hosts/watchos/swift/WatchApp.swift").is_file());
		assert!(root.join("hosts/watchos/guest.rs").is_file());
		let _ = std::fs::remove_dir_all(&root);
	}

	#[test]
	fn validate_requires_package_id() {
		let config = LeptoskitConfig::default();
		assert!(validate(&config).is_err());
	}

	#[test]
	fn scaffold_rejects_non_watch_targets() {
		let root = std::env::temp_dir().join(format!("leptoskit-watch-bad-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		let config = LeptoskitConfig {
			native: NativeConfig {
				package_id: "dev.leptoskit.steps".into(),
				..NativeConfig::default()
			},
			..LeptoskitConfig::default()
		};
		assert!(scaffold(&root, &config, NativeTarget::Android).is_err());
		let _ = std::fs::remove_dir_all(&root);
	}

	#[test]
	fn build_reports_experimental_for_each_watch_target() {
		let wear = build(Target::Wearos).unwrap_err().to_string();
		assert!(wear.contains("Wear OS"), "{wear}");
		let watch = build(Target::Watchos).unwrap_err().to_string();
		assert!(watch.contains("watchOS"), "{watch}");
		assert!(build(Target::Android).is_err());
	}

	#[test]
	fn write_reports_when_a_parent_directory_cannot_be_created() {
		let root =
			std::env::temp_dir().join(format!("leptoskit-watch-write-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		let blocker = root.join("blocked");
		std::fs::write(&blocker, b"not a directory").unwrap();
		let error = write(&blocker.join("child.kt"), "fn")
			.unwrap_err()
			.to_string();
		assert!(error.contains("could not create"), "{error}");
		let root_error = write(Path::new("/"), "fn").unwrap_err().to_string();
		assert!(
			root_error.contains("could not create parent"),
			"{root_error}"
		);
		let _ = std::fs::remove_dir_all(&root);
	}
}
