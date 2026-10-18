//! CLI tests: scaffolding, caching, and digests.

use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_cli::cache::BuildCache;
use leptoskit_cli::cache::Fingerprint;
use leptoskit_cli::cache::hash_bytes;
use leptoskit_cli::new_project;

struct Scratch {
	path: PathBuf,
}

impl Scratch {
	fn new(tag: &str) -> Self {
		static COUNTER: AtomicU64 = AtomicU64::new(0);
		let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
		let path = std::env::temp_dir().join(format!(
			"leptoskit_cli_test_{tag}_{}_{}",
			std::process::id(),
			unique
		));
		std::fs::create_dir_all(&path).expect("scratch");
		Self { path }
	}
}

impl Drop for Scratch {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.path);
	}
}

#[test]
fn scaffolds_a_project_with_config_and_source() {
	let scratch = Scratch::new("scaffold");
	let project = scratch.path.join("counter");
	new_project::run(
		project.to_string_lossy().as_ref(),
		leptoskit_config::Platform::Native,
		None,
	)
	.expect("scaffold");
	new_project::verify_scaffold(&project).expect("verify");

	let raw = std::fs::read_to_string(project.join("leptoskit.toml")).expect("read config");
	let config: leptoskit_config::LeptoskitConfig = toml::from_str(&raw).expect("parse config");
	assert_eq!(config.app.name, "counter");
	assert_eq!(
		config.app.default_platform,
		leptoskit_config::Platform::Native
	);
	assert!(config.native.package_id.contains("counter"));
	assert!(config.plugins.iter().any(|spec| spec.leptos));
}

#[test]
fn scaffolding_twice_fails_cleanly() {
	let scratch = Scratch::new("twice");
	let project = scratch.path.join("dupe");
	new_project::run(
		project.to_string_lossy().as_ref(),
		leptoskit_config::Platform::Web,
		None,
	)
	.expect("first scaffold");
	let error = new_project::run(
		project.to_string_lossy().as_ref(),
		leptoskit_config::Platform::Web,
		None,
	)
	.expect_err("second scaffold");
	assert!(error.to_string().contains("already exists"), "got {error}");
}

#[test]
fn fingerprints_change_with_inputs() {
	let scratch = Scratch::new("fingerprints");
	let file = scratch.path.join("input.txt");
	std::fs::write(&file, b"one").expect("write");

	let first = Fingerprint::new("phase", "tool 1").with_file("input", &file);
	let same = Fingerprint::new("phase", "tool 1").with_file("input", &file);
	assert_eq!(first.digest(), same.digest());

	std::fs::write(&file, b"two").expect("rewrite");
	let changed = Fingerprint::new("phase", "tool 1").with_file("input", &file);
	assert_ne!(first.digest(), changed.digest());

	let retargeted = Fingerprint::new("other", "tool 1").with_file("input", &file);
	assert_ne!(changed.digest(), retargeted.digest());
}

#[test]
fn build_cache_round_trips_and_skips_unchanged_phases() {
	let scratch = Scratch::new("cache");
	let mut cache = BuildCache::load(&scratch.path);
	let fingerprint = Fingerprint::new("phase", "tool");
	assert!(!cache.is_current(&fingerprint));
	cache.record(&fingerprint);
	assert!(cache.is_current(&fingerprint));
	cache.save().expect("save");

	let reloaded = BuildCache::load(&scratch.path);
	assert!(reloaded.is_current(&fingerprint));
	assert!(!reloaded.is_current(&Fingerprint::new("other", "tool")));
}

#[test]
fn directory_digest_tracks_content_not_names_alone() {
	let scratch = Scratch::new("digest");
	let dir = scratch.path.join("tree");
	std::fs::create_dir_all(dir.join("nested")).expect("mkdir");
	std::fs::write(dir.join("a.txt"), b"alpha").expect("write");
	std::fs::write(dir.join("nested").join("b.txt"), b"beta").expect("write");
	let before = leptoskit_cli::build::directory_digest(&dir);
	std::fs::write(dir.join("nested").join("b.txt"), b"gamma").expect("rewrite");
	let after = leptoskit_cli::build::directory_digest(&dir);
	assert_ne!(before, after);
	assert_eq!(before.len(), hash_bytes(b"x").len());
}

#[test]
fn scaffold_files_are_the_promised_set() {
	assert_eq!(
		new_project::template_files(),
		&[leptoskit_config::CONFIG_FILE, "src/main.rs"]
	);
	let _ = Path::new(".");
}
