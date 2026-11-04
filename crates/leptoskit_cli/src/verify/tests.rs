use super::*;

fn bundle() -> tempfile::TempDir {
	let root = tempfile::tempdir().unwrap();
	for path in ["source/src", "dependencies/example", "infra", "artifacts"] {
		std::fs::create_dir_all(root.path().join(path)).unwrap();
	}
	for (path, contents) in [
		(
			"source/Cargo.toml",
			"[package]\nname = 'backend'\nversion = '0.1.0'\n",
		),
		("source/Cargo.lock", "version = 4\n"),
		("source/src/main.rs", "fn main() {}\n"),
		("dependencies/example/lib.rs", "pub const VALUE: u32 = 1;\n"),
		("infra/main.tf", "# deployment definition\n"),
		("artifacts/backend", "a backend executable"),
		("secrets.json", "[\"DATABASE_URL\", \"API_KEY\"]"),
	] {
		std::fs::write(root.path().join(path), contents).unwrap();
	}
	write_recipe(root.path(), &recipe());
	root
}

fn recipe() -> serde_json::Value {
	serde_json::json!({
		"repository": "https://github.com/example/backend",
		"commit": "1234567890123456789012345678901234567890",
		"builder_image": format!("rust@sha256:{}", "a".repeat(64)),
		"target": "x86_64-unknown-linux-gnu",
		"package": "backend",
		"binary": "backend",
		"features": [],
		"no_default_features": false,
		"rustflags": []
	})
}

fn write_recipe(root: &Path, value: &serde_json::Value) {
	std::fs::write(root.join("build.json"), serde_json::to_vec(value).unwrap()).unwrap();
}

fn pin(root: &Path) -> (tempfile::NamedTempFile, String) {
	let mut file = tempfile::NamedTempFile::new().unwrap();
	let bytes = serde_json::to_vec_pretty(&inventory(root).unwrap()).unwrap();
	file.write_all(&bytes).unwrap();
	(file, hex::encode(Sha256::digest(bytes)))
}

#[test]
fn a_pinned_inventory_covers_every_role_and_has_stable_serialization() {
	let root = bundle();
	let (manifest, digest) = pin(root.path());
	let entries = check(root.path(), manifest.path(), &digest).unwrap();
	assert_eq!(entries, inventory(root.path()).unwrap().entries.len());
	assert_eq!(
		std::fs::read(manifest.path()).unwrap(),
		serde_json::to_vec_pretty(&inventory(root.path()).unwrap()).unwrap()
	);
	for path in [
		"source/Cargo.lock",
		"source/src/main.rs",
		"dependencies/example/lib.rs",
		"infra/main.tf",
		"artifacts/backend",
		"secrets.json",
	] {
		let path = root.path().join(path);
		let original = std::fs::read(&path).unwrap();
		std::fs::write(&path, b"changed").unwrap();
		assert!(
			check(root.path(), manifest.path(), &digest).is_err(),
			"{}",
			path.display()
		);
		std::fs::write(path, original).unwrap();
	}
	let mut changed = recipe();
	changed["target"] = "aarch64-unknown-linux-gnu".into();
	write_recipe(root.path(), &changed);
	assert!(check(root.path(), manifest.path(), &digest).is_err());
}

#[test]
fn extra_files_empty_directories_and_missing_files_are_rejected() {
	let root = bundle();
	let (manifest, digest) = pin(root.path());
	for path in [
		"source/injected.rs",
		"dependencies/extra",
		"infra/extra",
		"artifacts/extra",
	] {
		let path = root.path().join(path);
		std::fs::write(&path, b"extra code").unwrap();
		assert!(
			check(root.path(), manifest.path(), &digest)
				.unwrap_err()
				.to_string()
				.contains("unexpected bundle entry")
		);
		std::fs::remove_file(path).unwrap();
	}
	let directory = root.path().join("source/extra");
	std::fs::create_dir(&directory).unwrap();
	assert!(check(root.path(), manifest.path(), &digest).is_err());
	std::fs::remove_dir(directory).unwrap();
	std::fs::remove_file(root.path().join("dependencies/example/lib.rs")).unwrap();
	assert!(
		check(root.path(), manifest.path(), &digest)
			.unwrap_err()
			.to_string()
			.contains("missing or changed")
	);
}

#[test]
fn replacing_both_bundle_and_inventory_cannot_bypass_a_trusted_digest() {
	let root = bundle();
	let (manifest, digest) = pin(root.path());
	std::fs::write(root.path().join("artifacts/backend"), b"substituted server").unwrap();
	let bytes = serde_json::to_vec_pretty(&inventory(root.path()).unwrap()).unwrap();
	std::fs::write(manifest.path(), bytes).unwrap();
	let error = check(root.path(), manifest.path(), &digest)
		.unwrap_err()
		.to_string();
	assert!(error.contains("independently pinned digest"));
	// The digest is checked before parsing untrusted metadata.
	std::fs::write(manifest.path(), b"not JSON").unwrap();
	assert!(
		check(root.path(), manifest.path(), &digest)
			.unwrap_err()
			.to_string()
			.contains("pinned digest")
	);
}

#[test]
fn missing_materials_mutable_builders_and_secret_values_are_rejected() {
	let root = bundle();
	let mut value = recipe();
	value["builder_image"] = "rust:latest".into();
	write_recipe(root.path(), &value);
	assert!(
		inventory(root.path())
			.unwrap_err()
			.to_string()
			.contains("pinned")
	);
	write_recipe(root.path(), &recipe());
	for contents in [
		"[\"API_KEY=value-that-must-not-be-printed\"]",
		"[\"API_KEY\",\"API_KEY\"]",
		"{\"API_KEY\":\"value\"}",
		"\"value-that-must-not-be-printed\"",
	] {
		std::fs::write(root.path().join("secrets.json"), contents).unwrap();
		let error = format!("{:#}", inventory(root.path()).unwrap_err());
		assert!(!error.contains("value-that-must-not-be-printed"));
	}
	std::fs::write(root.path().join("secrets.json"), "[]").unwrap();
	std::fs::remove_file(root.path().join("source/Cargo.lock")).unwrap();
	assert!(
		inventory(root.path())
			.unwrap_err()
			.to_string()
			.contains("source/Cargo.lock")
	);
}

#[test]
fn unknown_formats_and_undeclared_top_level_entries_are_rejected() {
	let root = bundle();
	let mut value = inventory(root.path()).unwrap();
	value.format_version += 1;
	let bytes = serde_json::to_vec(&value).unwrap();
	let digest = hex::encode(Sha256::digest(&bytes));
	let manifest = tempfile::NamedTempFile::new().unwrap();
	std::fs::write(manifest.path(), bytes).unwrap();
	assert!(
		check(root.path(), manifest.path(), &digest)
			.unwrap_err()
			.to_string()
			.contains("unsupported")
	);
	std::fs::write(root.path().join(".env"), b"private data").unwrap();
	assert!(
		inventory(root.path())
			.unwrap_err()
			.to_string()
			.contains("top-level")
	);
}

#[test]
fn recording_never_overwrites_a_file_or_inventories_its_own_output() {
	let root = bundle();
	let out = tempfile::NamedTempFile::new().unwrap();
	std::fs::write(out.path(), b"keep me").unwrap();
	assert!(
		run(VerifyCommand::Record {
			bundle: root.path().to_owned(),
			out: out.path().to_owned()
		})
		.is_err()
	);
	assert_eq!(std::fs::read(out.path()).unwrap(), b"keep me");
	assert!(
		run(VerifyCommand::Record {
			bundle: root.path().to_owned(),
			out: root.path().join("source/inventory.json")
		})
		.is_err()
	);
	assert!(!root.path().join("source/inventory.json").exists());
}

#[cfg(unix)]
#[test]
fn symlinks_and_changed_permissions_cannot_escape_the_inventory() {
	use std::os::unix::fs::PermissionsExt as _;
	use std::os::unix::fs::symlink;

	let root = bundle();
	let (manifest, digest) = pin(root.path());
	let link = root.path().join("source/external");
	symlink(manifest.path(), &link).unwrap();
	assert!(
		inventory(root.path())
			.unwrap_err()
			.to_string()
			.contains("links are not supported")
	);
	std::fs::remove_file(&link).unwrap();
	symlink(root.path().join("dependencies"), &link).unwrap();
	assert!(inventory(root.path()).is_err());
	std::fs::remove_file(link).unwrap();
	let binary = root.path().join("artifacts/backend");
	let mode = std::fs::metadata(&binary).unwrap().permissions().mode();
	std::fs::set_permissions(binary, std::fs::Permissions::from_mode(mode ^ 0o100)).unwrap();
	assert!(check(root.path(), manifest.path(), &digest).is_err());
}
