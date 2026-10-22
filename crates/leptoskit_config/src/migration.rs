//! Explicit, comment-preserving upgrades of `leptoskit.toml`.

use toml_edit::DocumentMut;
use toml_edit::Item;
use toml_edit::Table;
use toml_edit::value;

use crate::ConfigError;
use crate::LeptoskitConfig;
use crate::SCHEMA_VERSION;
use crate::schema_url;

/// A migration fails before the caller writes anything.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
	/// The input is not valid TOML.
	#[error("could not parse configuration: {0}")]
	Parse(#[from] toml_edit::TomlError),
	/// Versions are non-negative integers; missing means legacy version 0.
	#[error("schema_version must be a non-negative integer")]
	InvalidVersion,
	/// An older CLI must never rewrite a newer contract.
	#[error(
		"schema_version {0} is newer than {SCHEMA_VERSION}; upgrade leptoskit before migrating"
	)]
	FutureVersion(u32),
	/// Every supported upgrade requires an explicit step.
	#[error("no configuration migration registered from schema_version {0}")]
	MissingStep(u32),
	/// Both locations are set; choosing one could lose the user's intent.
	#[error("both build.wasm_opt and bundle.wasm_opt are set; remove one before migrating")]
	ConflictingWasmOpt,
	/// The destination must be a table.
	#[error("bundle must be a table before build.wasm_opt can move into it")]
	InvalidBundle,
	/// Unknown fields and malformed values are never discarded by migration.
	#[error("migrated configuration is invalid: {0}")]
	InvalidConfig(#[from] toml::de::Error),
	/// Cross-field constraints still apply after migration.
	#[error(transparent)]
	Validation(#[from] ConfigError),
}

/// Upgrade a configuration to the current contract without writing a file.
///
/// Unversioned files start at version 0. Each step upgrades exactly one
/// version; published steps stay in this chain when a new major is added.
/// The result is validated, keeps unrelated settings and comments, and is
/// byte-for-byte stable on a second invocation.
pub fn migrate(raw: &str) -> Result<String, MigrationError> {
	let raw = without_schema_directive(raw);
	let mut document: DocumentMut = raw.parse()?;
	let mut version = match document.get("schema_version") {
		None => 0,
		Some(item) => {
			item.as_integer()
				.and_then(|version| u32::try_from(version).ok())
				.ok_or(MigrationError::InvalidVersion)?
		}
	};

	if version > SCHEMA_VERSION {
		return Err(MigrationError::FutureVersion(version));
	}

	while version < SCHEMA_VERSION {
		match version {
			0 => v0_to_v1(&mut document)?,
			other => return Err(MigrationError::MissingStep(other)),
		}

		version += 1;
		let mut stamp = value(i64::from(version));

		if let Some(previous) = document.get("schema_version").and_then(Item::as_value) {
			if let Some(stamp) = stamp.as_value_mut() {
				*stamp.decor_mut() = previous.decor().clone();
			}
		}

		document["schema_version"] = stamp;
	}

	let output = format!("#:schema {}\n{document}", schema_url());
	let config: LeptoskitConfig = toml::from_str(&output)?;
	config.validate()?;

	Ok(output)
}

fn v0_to_v1(document: &mut DocumentMut) -> Result<(), MigrationError> {
	let Some((key, wasm_opt)) = document
		.get_mut("build")
		.and_then(Item::as_table_like_mut)
		.and_then(|build| {
			let key = build.key("wasm_opt")?.clone();
			Some((key, build.remove("wasm_opt")?))
		})
	else {
		return Ok(());
	};
	let bundle = document
		.entry("bundle")
		.or_insert_with(|| Item::Table(Table::new()))
		.as_table_like_mut()
		.ok_or(MigrationError::InvalidBundle)?;

	if bundle.contains_key("wasm_opt") {
		return Err(MigrationError::ConflictingWasmOpt);
	}

	bundle.entry_format(&key).or_insert(wasm_opt);
	Ok(())
}

fn without_schema_directive(raw: &str) -> String {
	let mut output = String::new();
	let mut in_header = true;

	for line in raw.split_inclusive('\n') {
		let trimmed = line.trim();

		if !trimmed.is_empty() && !trimmed.starts_with('#') {
			in_header = false;
		}

		if in_header
			&& trimmed
				.strip_prefix('#')
				.is_some_and(|comment| comment.trim_start().starts_with(":schema "))
		{
			continue;
		}

		output.push_str(line);
	}

	output
}
