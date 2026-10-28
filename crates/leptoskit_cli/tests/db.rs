//! `leptoskit db` end to end: a throwaway app whose db crate depends on this
//! repository's `leptoskit_db`, migrated through the real binary.
//!
//! Ignored by default because it compiles the app; CI runs it with the other
//! end-to-end tests (`--ignored`).

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

const TABLES: &str = r#"
use leptoskit_db::Id;
use leptoskit_db::table;

/// Someone with an account.
#[table(name = "users", public)]
pub struct User {
	pub id: Id<Self>,
	#[db(unique, max_len = 32)]
	pub handle: String,
	// EXTRA
}

leptoskit_db::export_schema!();
"#;

fn repository() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR"))
		.join("../..")
		.canonicalize()
		.unwrap()
}

/// A workspace with one db crate, `app_db`, and a SQLite `[db]` table.
fn app(root: &Path) {
	let leptoskit_db = repository().join("crates/leptoskit_db");
	fs::write(
		root.join("Cargo.toml"),
		"[workspace]\nmembers = [\"app_db\"]\nresolver = \"2\"\n",
	)
	.unwrap();
	fs::create_dir_all(root.join("app_db/src")).unwrap();
	fs::write(
		root.join("app_db/Cargo.toml"),
		format!(
			"[package]\nname = \"app_db\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = \
			 false\n\n[dependencies]\nleptoskit_db = {{ path = {:?} }}\n\n[features]\nserver = \
			 [\"leptoskit_db/server\"]\n",
			leptoskit_db.display().to_string()
		),
	)
	.unwrap();
	fs::write(root.join("app_db/src/lib.rs"), TABLES).unwrap();
	fs::write(
		root.join("leptoskit.toml"),
		"schema_version = 0\n\n[app]\nname = \"app\"\nversion = \"0.1.0\"\n\n[db]\ndialect = \
		 \"sqlite\"\n",
	)
	.unwrap();
	// The repository's lock file pins the same versions it tests with.
	fs::copy(repository().join("Cargo.lock"), root.join("Cargo.lock")).unwrap();
}

fn leptoskit(root: &Path, args: &[&str]) -> Output {
	let target = std::env::var_os("LEPTOSKIT_E2E_TARGET_DIR")
		.map_or_else(|| root.join("target"), PathBuf::from);
	Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args(args)
		.current_dir(root)
		.env("CARGO_TARGET_DIR", target)
		.env_remove("DATABASE_URL")
		.output()
		.expect("run leptoskit")
}

fn succeeds(output: &Output) -> String {
	let stdout = String::from_utf8_lossy(&output.stdout).to_string();
	assert!(
		output.status.success(),
		"leptoskit failed:\n{stdout}\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
	stdout
}

fn fails(output: &Output) -> String {
	assert!(!output.status.success(), "leptoskit should have failed");
	String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
#[ignore = "compiles a throwaway app; run with --ignored"]
fn migrations_follow_the_tables() {
	let root = tempfile::tempdir().unwrap();
	app(root.path());

	let created = succeeds(&leptoskit(
		root.path(),
		&["db", "migration", "create", "init"],
	));
	assert!(created.contains("+ table users (2 columns)"), "{created}");
	assert!(succeeds(&leptoskit(root.path(), &["db", "check"])).contains("the tables match"));

	let grown = TABLES.replace("// EXTRA", "pub bio: Option<String>,");
	fs::write(root.path().join("app_db/src/lib.rs"), grown).unwrap();
	let drifted = fails(&leptoskit(root.path(), &["db", "check"]));
	assert!(drifted.contains("+ column users.bio (text)"), "{drifted}");

	std::thread::sleep(std::time::Duration::from_secs(1));
	succeeds(&leptoskit(
		root.path(),
		&["db", "migration", "create", "add bio"],
	));
	succeeds(&leptoskit(root.path(), &["db", "check"]));

	let url = format!("sqlite://{}", root.path().join("app.sqlite3").display());
	let migrated = succeeds(&leptoskit(root.path(), &["db", "migrate", "--url", &url]));
	assert_eq!(
		migrated
			.lines()
			.filter(|line| line.starts_with("applied "))
			.count(),
		2,
		"{migrated}"
	);
	assert!(
		succeeds(&leptoskit(root.path(), &["db", "migrate", "--url", &url])).contains("up to date")
	);
	let status = succeeds(&leptoskit(root.path(), &["db", "status", "--url", &url]));
	assert!(
		status.contains("the database has every migration"),
		"{status}"
	);

	assert!(fails(&leptoskit(root.path(), &["db", "reset", "--url", &url])).contains("pass --yes"));
	assert!(
		succeeds(&leptoskit(
			root.path(),
			&["db", "reset", "--url", &url, "--yes"]
		))
		.contains("applied 2")
	);
	assert!(fails(&leptoskit(root.path(), &["db", "migrate"])).contains("set DATABASE_URL"));
}

#[test]
fn the_database_must_be_configured() {
	let root = tempfile::tempdir().unwrap();
	fs::write(
		root.path().join("leptoskit.toml"),
		"schema_version = 0\n\n[app]\nname = \"app\"\nversion = \"0.1.0\"\n",
	)
	.unwrap();
	let error = fails(&leptoskit(root.path(), &["db", "check"]));
	assert!(error.contains("add a `[db]` table"), "{error}");
}
