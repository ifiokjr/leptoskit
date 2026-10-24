mod android;
mod ios;
pub(super) mod platform_sources;
mod registrant;

use std::path::Path;

use anyhow::Context;
use anyhow::Result;
pub(super) use registrant::rust_registrant;

use super::Target;
use super::registry::Registry;

/// Validate destinations before writing so unsupported plugins cannot leave a
/// partial host.
pub fn generate(
	root: &Path,
	config: &leptoskit_config::LeptoskitConfig,
	registry: &Registry,
	target: Target,
	_release: bool,
) -> Result<()> {
	if matches!(target, Target::Windows | Target::Linux) {
		return super::rust_desktop::generate(root, config, registry, target);
	}

	if matches!(target, Target::Harmonyos | Target::HarmonyosX86_64) {
		super::harmonyos::validate_plugins(registry)?;
	}

	if matches!(target, Target::Ios | Target::IosSimulator) {
		validate_ios_usage_descriptions(root, registry)?;
	}

	let generated = super::generated_dir(root, target);

	if let Ok(metadata) = std::fs::symlink_metadata(&generated) {
		anyhow::ensure!(
			metadata.is_dir() && !metadata.file_type().is_symlink(),
			".leptoskit must be an ordinary generated directory"
		);
	}

	std::fs::create_dir_all(&generated)?;
	write(
		&generated.join("registrant.rs"),
		&rust_registrant(registry, target)?,
	)?;

	match target {
		Target::Macos => super::desktop::generate(root, config, registry),
		Target::Windows | Target::Linux => unreachable!("desktop registrant generated above"),
		Target::Ios | Target::IosSimulator => ios::generate(root, config, registry),
		Target::Android => android::generate(root, registry, "arm64-v8a"),
		Target::AndroidX86_64 => android::generate(root, registry, "x86_64"),
		Target::Harmonyos | Target::HarmonyosX86_64 => {
			super::harmonyos::generate(root, config, registry, target)
		}
	}
}

fn validate_ios_usage_descriptions(root: &Path, registry: &Registry) -> Result<()> {
	let required: std::collections::BTreeSet<_> = registry
		.plugins
		.iter()
		.flat_map(|plugin| &plugin.apple_usage_descriptions)
		.collect();

	if required.is_empty() {
		return Ok(());
	}

	let path = super::host_dir(root, Target::Ios).join("Info.plist");
	let plist = plist::Value::from_file(&path).with_context(|| {
		format!(
			"read app-owned iOS usage descriptions from {}",
			path.display()
		)
	})?;
	let dictionary = plist
		.as_dictionary()
		.context("iOS Info.plist must contain a property-list dictionary")?;

	for key in required {
		anyhow::ensure!(
			dictionary
				.get(key)
				.and_then(plist::Value::as_string)
				.is_some_and(|text| !text.trim().is_empty() && !text.contains('\0')),
			"selected iOS plugins require a nonempty app-owned string for `{key}` in {}; describe \
			 your app's actual use before building",
			path.display()
		);
	}

	Ok(())
}
/// Replace copied packages so removed sources cannot remain in a generated
/// host.
pub(super) fn copy_package(source: &Path, destination: &Path) -> Result<()> {
	ensure_directory(destination)?;

	for entry in std::fs::read_dir(destination)? {
		let entry = entry?;

		if entry.file_type()?.is_dir() {
			std::fs::remove_dir_all(entry.path())?;
		} else {
			std::fs::remove_file(entry.path())?;
		}
	}

	for entry in walkdir::WalkDir::new(source)
		.into_iter()
		.filter_entry(|entry| {
			!matches!(
				entry.file_name().to_str(),
				Some("target" | ".build" | ".git" | ".swiftpm" | ".leptoskit" | "node_modules")
			)
		}) {
		let entry = entry?;
		anyhow::ensure!(
			!entry.file_type().is_symlink(),
			"native package source cannot be a symlink: {}",
			entry.path().display()
		);

		if entry.file_type().is_file() {
			copy(
				entry.path(),
				&destination.join(entry.path().strip_prefix(source)?),
			)?;
		}
	}
	Ok(())
}
fn quote(path: &Path) -> Result<String> {
	Ok(serde_json::to_string(
		path.to_str().context("native build path is not UTF-8")?,
	)?)
}
pub(super) fn write(path: &Path, content: &str) -> Result<()> {
	check_output(path)?;

	if std::fs::read(path).is_ok_and(|existing| existing == content.as_bytes()) {
		return Ok(());
	}

	std::fs::write(path, content).with_context(|| format!("write {}", path.display()))
}

pub fn copy(from: &Path, to: &Path) -> Result<()> {
	check_output(to)?;
	std::fs::copy(from, to)
		.with_context(|| format!("copy {} to {}", from.display(), to.display()))?;
	Ok(())
}

fn check_output(path: &Path) -> Result<()> {
	ensure_directory(path.parent().context("generated file has no parent")?)?;

	if let Ok(metadata) = std::fs::symlink_metadata(path) {
		anyhow::ensure!(
			metadata.is_file() && !metadata.file_type().is_symlink(),
			"generated output must be a regular file: {}",
			path.display()
		);
	}

	Ok(())
}

fn ensure_directory(path: &Path) -> Result<()> {
	for parent in path.ancestors() {
		if let Ok(metadata) = std::fs::symlink_metadata(parent) {
			anyhow::ensure!(
				metadata.is_dir() && !metadata.file_type().is_symlink(),
				"generated directory must not traverse a symlink: {}",
				parent.display()
			);
		}
	}

	std::fs::create_dir_all(path)?;
	Ok(())
}

#[cfg(test)]
mod tests;
