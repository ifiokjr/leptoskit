//! The explicit configuration upgrade command.

use std::io::Write;
use std::path::Path;

use anyhow::Context;
use leptoskit_config::CONFIG_FILE;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::SCHEMA_VERSION;
use leptoskit_config::migration;

/// Preview, check, or atomically write a validated configuration migration.
pub fn run(file: Option<&Path>, dry_run: bool, check: bool) -> anyhow::Result<()> {
	let path = match file {
		Some(path) => path.to_owned(),
		None => LeptoskitConfig::find_root()?.join(CONFIG_FILE),
	};
	let raw = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
	let output = migration::migrate(&raw).with_context(|| format!("migrate {}", path.display()))?;

	if dry_run {
		print!("{output}");
		return Ok(());
	}

	if raw == output {
		println!(
			"{} is current (schema_version {SCHEMA_VERSION})",
			path.display()
		);
		return Ok(());
	}

	anyhow::ensure!(
		!check,
		"{} needs migration; run `leptoskit migrate`",
		path.display()
	);
	let metadata = std::fs::symlink_metadata(&path)?;
	anyhow::ensure!(
		!metadata.file_type().is_symlink(),
		"{} is a symlink; pass its target with --file",
		path.display()
	);
	let parent = path
		.parent()
		.filter(|parent| !parent.as_os_str().is_empty())
		.unwrap_or(Path::new("."));
	let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
	temporary.write_all(output.as_bytes())?;
	temporary
		.as_file()
		.set_permissions(metadata.permissions())?;
	temporary.as_file().sync_all()?;
	anyhow::ensure!(
		std::fs::read_to_string(&path)? == raw,
		"{} changed during migration; run the command again",
		path.display()
	);
	temporary
		.persist(&path)
		.with_context(|| format!("replace {}", path.display()))?;
	println!(
		"migrated {} to schema_version {SCHEMA_VERSION}",
		path.display()
	);
	Ok(())
}
