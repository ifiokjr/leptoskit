//! Historical configuration upgrades and failure behavior.

use leptoskit_config::LeptoskitConfig;
use leptoskit_config::SCHEMA_VERSION;
use leptoskit_config::migration::MigrationError;
use leptoskit_config::migration::migrate;
use leptoskit_config::schema_url;

#[test]
fn upgrades_legacy_files_and_keeps_settings_and_comments() {
	let fixture = include_str!("fixtures/legacy.toml").replace("\r\n", "\n");

	for newline in ["\n", "\r\n"] {
		assert_legacy_migration(&fixture.replace('\n', newline));
	}
}

fn assert_legacy_migration(raw: &str) {
	let original: toml::Table = toml::from_str(raw).expect("parse legacy values");
	let output = migrate(raw).expect("migrate legacy config");
	let config: LeptoskitConfig = toml::from_str(&output).expect("parse migrated config");
	config.validate().expect("validate migrated config");
	assert_eq!(config.schema_version, SCHEMA_VERSION);
	assert!(!config.bundle.wasm_opt);
	assert_eq!(config.app.name, "legacy");
	assert_eq!(
		config.plugins[0].options["message"],
		original["plugins"][0]["options"]["message"]
	);
	assert!(output.starts_with(&format!("#:schema {}\n", schema_url())));

	for comment in [
		"# project identity",
		"# speed up local builds",
		"# keep this choice",
		"# plugin options",
	] {
		assert!(output.contains(comment), "lost {comment}:\n{output}");
	}

	assert_eq!(migrate(&output).expect("migrate twice"), output);
}

#[test]
fn current_files_are_byte_stable() {
	let raw = format!(
		"#:schema {}\n\nschema_version = 0\n\n[app]\nname = 'unchanged' # keep quotes\n",
		schema_url()
	);
	assert_eq!(migrate(&raw).expect("current"), raw);
}

#[test]
fn accepts_empty_and_current_unversioned_files() {
	for raw in [
		"",
		"schema_version = 0\n",
		"[bundle]\nwasm_opt = false\n",
		"[router]\n",
		"[l10n]\n",
		"build.wasm_opt = false\n",
		"build = { wasm_opt = false }\nbundle = { optimize = 'fast' }\n",
	] {
		let output = migrate(raw).expect("upgrade");
		let config: LeptoskitConfig = toml::from_str(&output).expect("parse");
		assert_eq!(config.schema_version, SCHEMA_VERSION);
		assert!(output.contains("schema_version = 0"));
		assert_eq!(migrate(&output).expect("repeat"), output);
	}
}

#[test]
fn rejects_future_invalid_and_conflicting_versions() {
	for version in [1, 2] {
		assert!(matches!(
			migrate(&format!("schema_version = {version}\n")),
			Err(MigrationError::FutureVersion(found)) if found == version
		));
	}

	for raw in [
		"schema_version = -1",
		"schema_version = '1'",
		"schema_version = 1.5",
		"schema_version = true",
		"schema_version = 4294967296",
	] {
		assert!(
			matches!(migrate(raw), Err(MigrationError::InvalidVersion)),
			"{raw}"
		);
	}

	assert!(matches!(
		migrate("[build]\nwasm_opt = true\n[bundle]\nwasm_opt = false\n"),
		Err(MigrationError::ConflictingWasmOpt)
	));
	assert!(migrate("bundle = false\n[build]\nwasm_opt = false\n").is_err());
}

#[test]
fn rejects_unknown_keys_and_invalid_values_without_dropping_them() {
	for raw in [
		"[broken",
		"unknown = true",
		"[app]\nrender = 'spa'",
		"[build]\nwasm_opt = 'no'",
		"schema_version = 0\n[build]\nwasm_opt = false",
		"[codepush]\nenabled = true",
		"[native]\npackage_id = 'bad'",
		"[router]\ndir = '../outside'",
		"[l10n]\noutput = 'src/l10n.txt'",
	] {
		assert!(migrate(raw).is_err(), "accepted {raw}");
	}
}

#[test]
fn updates_only_header_directives() {
	let raw = "# project\n# :schema old.json\n\n[app]\nname = 'demo'\n";
	let output = migrate(raw).expect("update directive");
	assert!(output.contains("# project\n"));
	assert!(!output.contains("old.json"));
	assert_eq!(output.matches("#:schema ").count(), 1);
}
