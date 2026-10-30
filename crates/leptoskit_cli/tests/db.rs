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

/// One HTTP/1.1 request to the scaffold's server, returning the status line's
/// code and the body.
fn request(address: &str, method: &str, path: &str, body: &str) -> (u16, String) {
	let (status, _, body) = exchange(address, method, path, "", body);
	(status, body)
}

/// One HTTP/1.1 request with extra header lines, each ending in `\r\n`,
/// returning the status line's code, the response headers, and the body.
fn exchange(
	address: &str,
	method: &str,
	path: &str,
	headers: &str,
	body: &str,
) -> (u16, String, String) {
	use std::io::Read;
	use std::io::Write;

	let mut stream = std::net::TcpStream::connect(address).expect("connect");
	write!(
		stream,
		"{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: \
		 application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
		body.len()
	)
	.unwrap();
	let mut response = String::new();
	stream.read_to_string(&mut response).unwrap();
	let status = response[9..12].parse().unwrap();
	let (head, body) = response.split_once("\r\n\r\n").unwrap_or((&response, ""));
	(status, head.to_owned(), body.to_owned())
}

/// `leptoskit new --db sqlite`, its first migration, and the server it
/// generates: migrated at startup, storing and listing notes.
#[test]
#[ignore = "compiles a scaffolded server; run with --ignored"]
fn the_database_scaffold_migrates_and_serves() {
	let scratch = tempfile::tempdir().unwrap();
	let sdk = repository();
	succeeds(&leptoskit(
		scratch.path(),
		&[
			"new",
			"notes",
			"--db",
			"sqlite",
			"--sdk-path",
			sdk.to_str().unwrap(),
		],
	));
	let root = scratch.path().join("notes");
	fs::copy(sdk.join("Cargo.lock"), root.join("Cargo.lock")).unwrap();

	let created = succeeds(&leptoskit(&root, &["db", "migration", "create", "init"]));
	assert!(created.contains("+ table notes (3 columns)"), "{created}");
	succeeds(&leptoskit(&root, &["db", "check"]));

	let target = std::env::var_os("LEPTOSKIT_E2E_TARGET_DIR")
		.map_or_else(|| root.join("target"), PathBuf::from);
	let built = Command::new(env!("CARGO"))
		.args(["build", "--locked", "--package", "notes"])
		.current_dir(&root)
		.env("CARGO_TARGET_DIR", &target)
		.status()
		.unwrap();
	assert!(built.success());

	let address = std::net::TcpListener::bind("127.0.0.1:0")
		.unwrap()
		.local_addr()
		.unwrap()
		.to_string();
	let mut server = Command::new(target.join("debug/notes"))
		.current_dir(&root)
		.env("LEPTOS_SITE_ADDR", &address)
		.env_remove("DATABASE_URL")
		.stdout(std::process::Stdio::null())
		.stderr(std::process::Stdio::null())
		.spawn()
		.unwrap();
	let ready = (0..120).any(|_| {
		std::thread::sleep(std::time::Duration::from_millis(250));
		std::net::TcpStream::connect(&address).is_ok()
	});
	let added = ready.then(|| request(&address, "POST", "/api/notes", r#"{"body":"hello"}"#));
	let empty = ready.then(|| request(&address, "POST", "/api/notes", r#"{"body":""}"#));
	let listed = ready.then(|| request(&address, "GET", "/api/notes", ""));
	server.kill().unwrap();
	server.wait().unwrap();

	assert!(ready, "the server never listened on {address}");
	let (status, body) = added.unwrap();
	assert_eq!(status, 200, "{body}");
	assert!(body.contains(r#""body":"hello""#), "{body}");
	assert_eq!(empty.unwrap().0, 422, "an empty note breaks min_len");
	let (status, body) = listed.unwrap();
	assert_eq!(status, 200);
	assert!(
		body.starts_with('[') && body.contains(r#""body":"hello""#),
		"{body}"
	);
	assert!(root.join("notes.sqlite3").is_file());
}

/// `leptoskit new --auth`: its tables migrate, the server and the app build
/// for the server, the browser, and a native guest, the generated tests pass,
/// and the server signs someone in with an emailed code from the development
/// mailbox.
#[test]
#[ignore = "compiles a scaffolded server; run with --ignored"]
fn the_auth_scaffold_signs_people_in() {
	let scratch = tempfile::tempdir().unwrap();
	let sdk = repository();
	succeeds(&leptoskit(
		scratch.path(),
		&[
			"new",
			"notes",
			"--auth",
			"--sdk-path",
			sdk.to_str().unwrap(),
		],
	));
	let root = scratch.path().join("notes");
	fs::copy(sdk.join("Cargo.lock"), root.join("Cargo.lock")).unwrap();

	let created = succeeds(&leptoskit(&root, &["db", "migration", "create", "init"]));
	assert!(created.contains("+ table users (3 columns)"), "{created}");
	assert!(created.contains("+ table auth.sessions"), "{created}");

	let target = std::env::var_os("LEPTOSKIT_E2E_TARGET_DIR")
		.map_or_else(|| root.join("target"), PathBuf::from);
	let cargo = |args: &[&str]| {
		let status = Command::new(env!("CARGO"))
			.args(args)
			.current_dir(&root)
			.env("CARGO_TARGET_DIR", &target)
			.status()
			.unwrap();
		assert!(status.success(), "cargo {}", args.join(" "));
	};
	cargo(&["build", "--locked", "--package", "notes"]);
	cargo(&["test", "--locked", "--package", "notes_app"]);
	for feature in ["hydrate", "native"] {
		cargo(&[
			"check",
			"--locked",
			"--package",
			"notes_app",
			"--target",
			"wasm32-unknown-unknown",
			"--features",
			feature,
		]);
	}

	let address = std::net::TcpListener::bind("127.0.0.1:0")
		.unwrap()
		.local_addr()
		.unwrap()
		.to_string();
	let origin = format!("Origin: http://{address}\r\n");
	let mut server = Command::new(target.join("debug/notes"))
		.current_dir(&root)
		.env("LEPTOS_SITE_ADDR", &address)
		.env(
			"AUTH_ENCRYPTION_KEYS",
			"test:BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc",
		)
		.env_remove("DATABASE_URL")
		.stdout(std::process::Stdio::null())
		.stderr(std::process::Stdio::null())
		.spawn()
		.unwrap();
	let ready = (0..120).any(|_| {
		std::thread::sleep(std::time::Duration::from_millis(250));
		std::net::TcpStream::connect(&address).is_ok()
	});
	let page = ready.then(|| request(&address, "GET", "/", ""));
	let sent = ready.then(|| {
		request(
			&address,
			"POST",
			"/auth/email/send",
			r#"{"email":"ada@example.com"}"#,
		)
	});
	let mail = ready.then(|| request(&address, "GET", "/_leptoskit/mail", ""));
	let code = mail.as_ref().and_then(|(_, body)| {
		let start = body.find("sign-in code: ")? + "sign-in code: ".len();
		Some(body[start..start + 9].to_owned())
	});
	let verified = code.as_ref().map(|code| {
		exchange(
			&address,
			"POST",
			"/auth/email/verify",
			&origin,
			&format!(r#"{{"email":"ada@example.com","code":"{code}"}}"#),
		)
	});
	let cookie = verified.as_ref().and_then(|(_, head, _)| {
		let line = head
			.lines()
			.find(|line| line.to_ascii_lowercase().starts_with("set-cookie:"))?;
		Some(
			line["set-cookie:".len()..]
				.split(';')
				.next()?
				.trim()
				.to_owned(),
		)
	});
	let session = cookie.as_ref().map(|cookie| {
		exchange(
			&address,
			"GET",
			"/auth/session",
			&format!("Cookie: {cookie}\r\n"),
			"",
		)
	});
	let notes = ready.then(|| request(&address, "POST", "/api/notes", r#"{"body":"hello"}"#));
	server.kill().unwrap();
	server.wait().unwrap();

	assert!(ready, "the server never listened on {address}");
	let (status, body) = page.unwrap();
	assert_eq!(status, 200);
	assert!(body.contains("Checking whether you"), "{body}");
	assert_eq!(sent.unwrap().0, 204);
	let code = code.unwrap_or_else(|| panic!("no code in the mailbox: {mail:?}"));
	let (status, _, body) = verified.unwrap();
	assert_eq!(status, 200, "{body}");
	assert!(body.contains(r#""signed_up":true"#), "{body}");
	assert!(cookie.is_some(), "no session cookie for {code}");
	let (status, _, body) = session.unwrap();
	assert_eq!(status, 200);
	assert!(body.contains(r#""account_id""#), "{body}");
	assert_eq!(
		notes.unwrap().0,
		200,
		"the notes still work under the auth layer"
	);

	assert_eq!(
		succeeds(&leptoskit(&root, &["generate", "auth"])),
		"nothing changed: the sign-in screens are already in place\n"
	);
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
