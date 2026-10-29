//! Explicit, comment-preserving upgrades of `leptoskit.toml`.

use toml_edit::Array;
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
	/// Versions are non-negative integers; missing means an unversioned legacy
	/// file.
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
	#[error(
		"both [router] and a [[plugins]] entry naming `router` are set; remove one before \
		 migrating"
	)]
	ConflictingRouter,
	/// The plugin list must be an array of tables for the router to join.
	#[error("plugins must be an array of tables, like `[[plugins]]`, for the router entry to join")]
	InvalidPlugins,
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
/// Unversioned legacy files migrate to version 0. Each later step upgrades
/// exactly one version; published steps stay in this chain when a new major is
/// added. The result is validated, keeps unrelated settings and comments, and
/// is byte-for-byte stable on a second invocation.
pub fn migrate(raw: &str) -> Result<String, MigrationError> {
	let raw = without_schema_directive(raw);
	let mut document: DocumentMut = raw.parse()?;
	let mut version = document
		.get("schema_version")
		.map(|item| {
			item.as_integer()
				.and_then(|version| u32::try_from(version).ok())
				.ok_or(MigrationError::InvalidVersion)
		})
		.transpose()?;

	if let Some(version) = version
		&& version > SCHEMA_VERSION
	{
		return Err(MigrationError::FutureVersion(version));
	}

	while version != Some(SCHEMA_VERSION) {
		let next = match version {
			None => {
				legacy_to_v0(&mut document)?;
				0
			}
			Some(other) => return Err(MigrationError::MissingStep(other)),
		};

		let mut stamp = value(i64::from(next));

		if let Some(previous) = document.get("schema_version").and_then(Item::as_value)
			&& let Some(stamp) = stamp.as_value_mut()
		{
			*stamp.decor_mut() = previous.decor().clone();
		}

		document["schema_version"] = stamp;
		version = Some(next);
	}

	router_to_plugin(&mut document)?;

	let output = format!("#:schema {}\n{document}", schema_url());
	let config: LeptoskitConfig = toml::from_str(&output)?;
	config.validate()?;

	Ok(output)
}

/// Moves the legacy `[router]` table into a `[[plugins]]` entry naming
/// `router`, keeping a non-default `dir` as the entry's one option. Absent
/// without `[router]`; refuses to choose when a router entry already exists.
fn router_to_plugin(document: &mut DocumentMut) -> Result<(), MigrationError> {
	let Some(router) = document.get_mut("router").and_then(Item::as_table_like_mut) else {
		return Ok(());
	};
	let dir = router.get("dir").and_then(Item::as_value).cloned();
	if let Some(dir) = dir
		.as_ref()
		.and_then(toml_edit::Value::as_str)
		.map(std::path::PathBuf::from)
		&& !(dir.is_relative()
			&& dir
				.components()
				.all(|component| matches!(component, std::path::Component::Normal(_))))
	{
		// The same rule validate() applies to a surviving `[router]` table;
		// moving the table must not smuggle an invalid path into options.
		return Err(MigrationError::Validation(ConfigError::InvalidRouterDir(
			dir,
		)));
	}
	document.remove("router");

	match document.get_mut("plugins") {
		None => {
			document
				.entry("plugins")
				.or_insert(Item::ArrayOfTables(toml_edit::ArrayOfTables::new()));
		}
		Some(Item::ArrayOfTables(_)) => {}
		Some(Item::Value(value)) if value.as_array().is_some_and(Array::is_empty) => {
			// An empty inline `plugins = []` becomes the array-of-tables the
			// router entry appends to.
			document.insert(
				"plugins",
				Item::ArrayOfTables(toml_edit::ArrayOfTables::new()),
			);
		}
		Some(_) => return Err(MigrationError::InvalidPlugins),
	}
	let Some(Item::ArrayOfTables(plugins)) = document.get_mut("plugins") else {
		return Err(MigrationError::InvalidPlugins);
	};
	if plugins.iter().any(|table| {
		table
			.get("name")
			.and_then(Item::as_value)
			.and_then(toml_edit::Value::as_str)
			== Some("router")
	}) {
		return Err(MigrationError::ConflictingRouter);
	}

	let mut entry = Table::new();
	entry["name"] = value("router");
	if let Some(dir) = dir
		&& dir.as_str()
			!= Some(
				crate::RouterConfig::default()
					.dir
					.to_str()
					.expect("utf-8 default"),
			) {
		let mut options = Table::new();
		options.insert("dir", Item::Value(dir.clone()));
		entry.insert("options", Item::Table(options));
	}
	plugins.push(entry);
	Ok(())
}

fn legacy_to_v0(document: &mut DocumentMut) -> Result<(), MigrationError> {
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
