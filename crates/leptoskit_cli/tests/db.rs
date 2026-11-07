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

/// A scaffolded server, killed if the test ends while it runs.
struct Server {
	child: std::process::Child,
}

impl Server {
	/// Starts the scaffold's debug server on `address`, with `envs`, and
	/// waits for it to listen.
	fn start(root: &Path, target: &Path, address: &str, envs: &[(&str, &str)]) -> Self {
		let mut child = Command::new(target.join("debug/notes"));
		child
			.current_dir(root)
			.env("LEPTOS_SITE_ADDR", address)
			.env_remove("DATABASE_URL")
			.stdout(std::process::Stdio::null())
			.stderr(std::process::Stdio::null());

		for (name, value) in envs {
			child.env(name, value);
		}

		let server = Self {
			child: child.spawn().unwrap(),
		};
		let ready = (0..240).any(|_| {
			std::thread::sleep(std::time::Duration::from_millis(250));
			std::net::TcpStream::connect(address).is_ok()
		});
		assert!(ready, "the server never listened on {address}");
		server
	}

	/// Stops the server the way a container runtime does, with `SIGTERM`, and
	/// returns whether it shut down cleanly: it fails `/readyz`, tells live
	/// sockets to reconnect, and finishes what's in flight.
	fn terminate(mut self) -> bool {
		#[cfg(unix)]
		{
			let signalled = Command::new("kill")
				.args(["-TERM", &self.child.id().to_string()])
				.status()
				.unwrap();
			assert!(signalled.success());
		}
		#[cfg(not(unix))]
		self.child.kill().unwrap();

		let stopped = (0..240).find_map(|_| {
			std::thread::sleep(std::time::Duration::from_millis(250));
			self.child.try_wait().unwrap()
		});
		stopped.is_some_and(|status| cfg!(not(unix)) || status.success())
	}
}

impl Drop for Server {
	fn drop(&mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

/// A free port on this machine, as `127.0.0.1:port`.
fn free_address() -> String {
	std::net::TcpListener::bind("127.0.0.1:0")
		.unwrap()
		.local_addr()
		.unwrap()
		.to_string()
}

/// Whether `program` runs; otherwise the part of the test that needs it is
/// skipped, unless `LEPTOSKIT_REQUIRE_SDK_TOOLCHAINS=1`.
fn have(program: &str) -> bool {
	let found = Command::new(program)
		.arg("--version")
		.output()
		.is_ok_and(|output| output.status.success());

	if !found {
		assert!(
			std::env::var("LEPTOSKIT_REQUIRE_SDK_TOOLCHAINS").as_deref() != Ok("1"),
			"`{program}` is required but isn't on PATH"
		);
		eprintln!("skipped: `{program}` isn't on PATH");
	}

	found
}

/// `leptoskit rpc spec` and `leptoskit rpc generate --target typescript`
/// with the server's environment broken: no sign-in keys, and a database
/// URL that can't connect. Both still succeed, because the server writes
/// the spec before it reads either, and nothing touches the database. The
/// spec lists `operations`, each at `POST /api/<name>`, and the backend's
/// `GET /api/health`; the client is written to `client`.
fn exports_without_the_environment(root: &Path, operations: &[&str], client: &Path) -> String {
	let target = std::env::var_os("LEPTOSKIT_E2E_TARGET_DIR")
		.map_or_else(|| root.join("target"), PathBuf::from);
	let broken = |args: &[&str]| {
		let output = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
			.args(args)
			.current_dir(root)
			.env("CARGO_TARGET_DIR", &target)
			.env("DATABASE_URL", "postgres://nobody@127.0.0.1:1/never")
			.env_remove("AUTH_ENCRYPTION_KEYS")
			.output()
			.expect("run leptoskit");
		succeeds(&output)
	};

	let spec = broken(&["rpc", "spec"]);
	let parsed: serde_json::Value = serde_json::from_str(&spec).expect("the spec is JSON");
	let functions = parsed["functions"]
		.as_array()
		.expect("the spec lists functions");
	let route = |name: &str| {
		functions
			.iter()
			.find(|function| function["name"] == name)
			.map(|function| {
				format!(
					"{} {}",
					function["method"].as_str().unwrap(),
					function["path"].as_str().unwrap()
				)
			})
	};
	assert_eq!(
		route("health").as_deref(),
		Some("GET /api/health"),
		"{spec}"
	);

	for name in operations {
		assert_eq!(
			route(name),
			Some(format!("POST /api/{name}")),
			"{name} isn't published: {spec}"
		);
	}

	broken(&[
		"rpc",
		"generate",
		"--target",
		"typescript",
		"--out",
		client.to_str().unwrap(),
	]);
	let index = fs::read_to_string(client.join("index.ts")).unwrap();
	assert!(index.contains("health(): Promise<Health>"), "{index}");

	for name in operations {
		let member = name
			.split('_')
			.enumerate()
			.map(|(index, part)| {
				if index == 0 {
					part.to_owned()
				} else {
					part[..1].to_uppercase() + &part[1..]
				}
			})
			.collect::<String>();
		assert!(
			index.contains(&format!("readonly {member} ")),
			"{member}: {index}"
		);
	}

	assert!(
		!root.join("notes.sqlite3").exists(),
		"exporting the spec opened the database"
	);
	spec
}

/// `leptoskit new --db sqlite`, its first migration, and the server it
/// generates: the published API exports without a database, the server
/// migrates at startup and serves the shared crate's typed operations, and
/// the TypeScript client generated from the spec, run outside this
/// repository, adds a note live, sees it in its subscription, and follows
/// the server through a restart.
#[test]
#[ignore = "compiles a scaffolded server; run with --ignored"]
fn the_database_scaffold_migrates_and_serves() {
	use std::io::BufRead;

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
	assert!(created.contains("+ table rpc.receipts"), "{created}");
	succeeds(&leptoskit(&root, &["db", "check"]));

	let client = scratch.path().join("client");
	exports_without_the_environment(&root, &["notes", "add_note"], &client);

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
	// The shared contracts build for the browser without the server's
	// driver.
	cargo(&[
		"check",
		"--locked",
		"--package",
		"notes_app",
		"--target",
		"wasm32-unknown-unknown",
		"--features",
		"hydrate",
	]);

	let address = free_address();
	let server = Server::start(&root, &target, &address, &[]);
	let health = request(&address, "GET", "/api/health", "");
	let added = request(&address, "POST", "/api/add_note", r#"{"body":"hello"}"#);
	let empty = request(&address, "POST", "/api/add_note", r#"{"body":""}"#);
	let listed = request(&address, "POST", "/api/notes", "{}");
	let ready = request(&address, "GET", "/readyz", "");

	assert_eq!(health, (200, r#"{"ready":true}"#.to_owned()));
	let (status, body) = added;
	assert_eq!(status, 200, "{body}");
	assert!(body.contains(r#""body":"hello""#), "{body}");
	let (status, body) = empty;
	assert_eq!(status, 400, "an empty note breaks min_len: {body}");
	assert!(body.starts_with("Args|"), "{body}");
	let (status, body) = listed;
	assert_eq!(status, 200);
	assert!(
		body.starts_with('[') && body.contains(r#""body":"hello""#),
		"{body}"
	);
	assert_eq!(ready.0, 200, "{}", ready.1);

	if have("node") {
		fs::copy(
			Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notes_sdk/journey.ts"),
			client.join("journey.ts"),
		)
		.unwrap();
		let mut journey = Command::new("node")
			.args(["--experimental-strip-types", "--no-warnings", "journey.ts"])
			.current_dir(&client)
			.env("NOTES_URL", format!("http://{address}"))
			.stdout(std::process::Stdio::piped())
			.stderr(std::process::Stdio::inherit())
			.spawn()
			.unwrap();
		let mut lines = std::io::BufReader::new(journey.stdout.take().unwrap()).lines();
		let mut said = Vec::new();
		let mut server = Some(server);

		for line in lines.by_ref() {
			let line = line.unwrap();
			said.push(line.clone());

			if line == "ready for restart" {
				assert!(
					server.take().unwrap().terminate(),
					"the server didn't shut down cleanly"
				);
				server = Some(Server::start(&root, &target, &address, &[]));
				let (status, body) = request(
					&address,
					"POST",
					"/api/add_note",
					r#"{"body":"after restart"}"#,
				);
				assert_eq!(status, 200, "{body}");
			}
		}

		let finished = journey.wait().unwrap();
		assert!(finished.success(), "the journey failed after {said:?}");
		assert_eq!(
			said,
			[
				"ok subscribed",
				"ok mutation",
				"ready for restart",
				"ok reconnect"
			]
		);
		assert!(server.unwrap().terminate());
	} else {
		assert!(server.terminate(), "the server didn't shut down cleanly");
	}

	assert!(root.join("notes.sqlite3").is_file());
}

/// `leptoskit db studio` on a throwaway app: it builds the studio into the
/// db crate, prints an address on this machine with a token, and serves
/// the app's tables there, only with the token.
#[test]
#[ignore = "compiles a throwaway app with the studio; run with --ignored"]
fn the_studio_serves_the_apps_tables() {
	use std::io::BufRead;

	let root = tempfile::tempdir().unwrap();
	app(root.path());
	succeeds(&leptoskit(
		root.path(),
		&["db", "migration", "create", "init"],
	));
	let database = root.path().join("app.sqlite3");
	let url = format!("sqlite://{}", database.display());
	succeeds(&leptoskit(root.path(), &["db", "migrate", "--url", &url]));

	let target = std::env::var_os("LEPTOSKIT_E2E_TARGET_DIR")
		.map_or_else(|| root.path().join("target"), PathBuf::from);
	let mut studio = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args(["db", "studio", "--url", &url])
		.current_dir(root.path())
		.env("CARGO_TARGET_DIR", target)
		.stdout(std::process::Stdio::piped())
		.stderr(std::process::Stdio::piped())
		.spawn()
		.unwrap();
	let stdout = std::io::BufReader::new(studio.stdout.take().unwrap());
	let address = stdout.lines().map_while(Result::ok).find_map(|line| {
		line.strip_prefix("leptoskit db studio: ")
			.map(ToString::to_string)
	});
	let pages = address.as_ref().map(|address| {
		let (host, path) = address
			.strip_prefix("http://")
			.and_then(|rest| rest.split_once('/'))
			.unwrap();
		(
			request(host, "GET", &format!("/{path}"), ""),
			request(host, "GET", "/", ""),
			request(
				host,
				"GET",
				&format!("/{}", path.replacen('?', "schema?", 1)),
				"",
			),
		)
	});
	studio.kill().unwrap();
	let output = studio.wait_with_output().unwrap();

	let address = address.unwrap_or_else(|| {
		panic!(
			"the studio never printed its address:\n{}",
			String::from_utf8_lossy(&output.stderr)
		)
	});
	assert!(address.starts_with("http://127.0.0.1:"), "{address}");
	assert!(address.contains("/?token="), "{address}");
	let (tables, without_token, schema) = pages.unwrap();
	assert_eq!(tables.0, 200);
	assert!(tables.1.contains(">users</a>"), "{}", tables.1);
	assert_eq!(without_token.0, 403);
	assert!(schema.1.contains("<h2>users</h2>"), "{}", schema.1);
}

/// `leptoskit new --auth`: its tables migrate, its published API exports
/// without sign-in keys or a database, the server and the app build for the
/// server, the browser, and a native guest that loads in the native runtime,
/// the generated tests pass, the server signs someone in with an emailed code
/// from the development mailbox, and each person's notes are their own.
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
	assert!(created.contains("+ table notes (4 columns)"), "{created}");
	assert!(created.contains("+ table auth.sessions"), "{created}");
	exports_without_the_environment(
		&root,
		&["notes", "add_note"],
		&scratch.path().join("client"),
	);

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
	cargo(&[
		"check",
		"--locked",
		"--package",
		"notes_app",
		"--target",
		"wasm32-unknown-unknown",
		"--features",
		"hydrate",
	]);
	// The native guest is optimized, as `leptoskit build` makes it.
	cargo(&[
		"build",
		"--locked",
		"--release",
		"--package",
		"notes_app",
		"--target",
		"wasm32-unknown-unknown",
		"--features",
		"native",
	]);
	native_guest_runs(&target.join("wasm32-unknown-unknown/release/notes_app.wasm"));

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
	// Each person's notes are their own: the operations run as the caller,
	// and the table's policy shows them only their rows.
	let notes = cookie
		.as_ref()
		.map(|ada| each_persons_notes(&address, &origin, ada));
	// The probes answer without a session.
	let probe = ready.then(|| request(&address, "GET", "/readyz", ""));
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
	assert_their_own(notes.unwrap());
	assert_eq!(probe.unwrap().0, 200);

	assert_eq!(
		succeeds(&leptoskit(&root, &["generate", "auth"])),
		"nothing changed: the sign-in screens are already in place\n"
	);
}

/// What Ada, signed in with `ada`, Bob, signed in here, and someone signed
/// out get from the notes: the signed-out call, Ada's and Bob's adds, then
/// Ada's and Bob's lists.
fn each_persons_notes(address: &str, origin: &str, ada: &str) -> Option<[(u16, String); 5]> {
	let (_, head, _) = exchange(
		address,
		"POST",
		"/auth/dev/sign-in",
		origin,
		r#"{"email":"bob@example.com"}"#,
	);
	let line = head
		.lines()
		.find(|line| line.to_ascii_lowercase().starts_with("set-cookie:"))?;
	let bob = line["set-cookie:".len()..]
		.split(';')
		.next()?
		.trim()
		.to_owned();
	let call = |cookie: Option<&str>, path: &str, body: &str| {
		let headers = match cookie {
			Some(cookie) => format!("{origin}Cookie: {cookie}\r\n"),
			None => origin.to_owned(),
		};
		let (status, _, body) = exchange(address, "POST", path, &headers, body);
		(status, body)
	};
	Some([
		call(None, "/api/add_note", r#"{"body":"nobody's"}"#),
		call(Some(ada), "/api/add_note", r#"{"body":"ada's"}"#),
		call(Some(&bob), "/api/add_note", r#"{"body":"bob's"}"#),
		call(Some(ada), "/api/notes", "{}"),
		call(Some(&bob), "/api/notes", "{}"),
	])
}

/// Someone signed out is refused, and Ada and Bob each see only their own
/// notes.
fn assert_their_own(notes: Option<[(u16, String); 5]>) {
	let [anonymous, added, bobs, adas_list, bobs_list] =
		notes.expect("dev sign-in sets Bob's cookie");
	let (status, body) = anonymous;
	assert_eq!(status, 401, "{body}");
	let (status, body) = added;
	assert_eq!(status, 200, "{body}");
	assert!(body.contains(r#""body":"ada's""#), "{body}");
	assert_eq!(bobs.0, 200, "{}", bobs.1);
	let (status, body) = adas_list;
	assert_eq!(status, 200, "{body}");
	assert!(
		body.contains("ada's") && !body.contains("bob's"),
		"Ada sees only hers: {body}"
	);
	let (status, body) = bobs_list;
	assert_eq!(status, 200, "{body}");
	assert!(
		body.contains("bob's") && !body.contains("ada's"),
		"Bob sees only his: {body}"
	);
}

/// The `--auth` scaffold's native guest imports none of the browser APIs the
/// query cache and sign-in client use there (timers, the clock, `navigator`,
/// the window, `fetch`, event listeners), takes the host's clock, and starts
/// in the interpreter every native host runs, showing the sign-in screen.
///
/// `leptos` itself still links js-sys's futures and stream glue; the
/// interpreter links those to traps, since nothing a host calls reaches them.
fn native_guest_runs(path: &Path) {
	let wasm = fs::read(path).unwrap();
	let engine = wasmi::Engine::default();
	let module = wasmi::Module::new(&engine, &wasm).unwrap();
	let browser: Vec<String> = module
		.imports()
		.map(|import| format!("{}::{}", import.module(), import.name()))
		.filter(|import| {
			[
				"setTimeout",
				"clearTimeout",
				"Date",
				"navigator",
				"Window",
				"fetch",
				"addEventListener",
				"visibilityState",
				"BroadcastChannel",
			]
			.iter()
			.any(|api| import.contains(api))
		})
		.collect();
	assert!(browser.is_empty(), "browser imports: {browser:?}");
	let modules: std::collections::BTreeSet<String> = module
		.imports()
		.map(|import| import.module().to_owned())
		.collect();
	assert!(
		modules.iter().all(|module| {
			["leptoskit", "leptoskit_plugins_v1"].contains(&module.as_str())
				|| module.starts_with("__wbindgen")
		}),
		"imports from {modules:?}"
	);
	assert!(
		module
			.exports()
			.any(|export| export.name() == "leptoskit_clock"),
		"the guest takes the host's clock"
	);

	let mut host = leptoskit_wasm::WasmHost::headless(&wasm).expect("the guest loads");
	host.start(390.0, 844.0).expect("the guest starts");
	let surface = host.surface();
	let mut texts = Vec::new();
	let mut nodes: Vec<_> = surface.root().into_iter().collect();

	while let Some(node) = nodes.pop() {
		texts.extend(surface.props(node).text.clone());
		nodes.extend(surface.children(node).iter().copied());
	}

	assert!(
		texts.iter().any(|text| text.contains("Checking")),
		"the sign-in screen: {texts:?}"
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

/// The directory holding an installed Postgres's `initdb`, found where the
/// database crate's tests and `dev_server::test_binaries` look:
/// `LEPTOSKIT_POSTGRES_BIN` or `PGBIN`, the path, or the newest of Debian's
/// `/usr/lib/postgresql/*/bin`, which is where CI's Ubuntu runners keep it.
/// `None` skips the test, unless CI requires Postgres.
fn postgres_binaries() -> Option<PathBuf> {
	let has_initdb = |directory: &Path| {
		directory.join("initdb").exists() || directory.join("initdb.exe").exists()
	};
	let from_env = ["LEPTOSKIT_POSTGRES_BIN", "PGBIN"]
		.into_iter()
		.filter_map(std::env::var_os)
		.map(PathBuf::from);
	let from_path = std::env::var_os("PATH")
		.into_iter()
		.flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>());
	let mut debian: Vec<PathBuf> = fs::read_dir("/usr/lib/postgresql")
		.into_iter()
		.flatten()
		.filter_map(|entry| Some(entry.ok()?.path().join("bin")))
		.collect();
	debian.sort_by_key(|path| {
		path.parent()
			.and_then(|version| version.file_name()?.to_str()?.parse::<u32>().ok())
			.unwrap_or(0)
	});
	debian.reverse();
	let found = from_env
		.chain(from_path)
		.chain(debian)
		.find(|directory| has_initdb(directory));

	if found.is_none() {
		assert!(
			std::env::var_os("LEPTOSKIT_REQUIRE_POSTGRES").is_none(),
			"LEPTOSKIT_REQUIRE_POSTGRES is set, but no Postgres binaries were found"
		);
	}

	found
}

/// A Postgres project with no `DATABASE_URL`: `leptoskit db` runs the
/// development database itself, keeps its data between commands, and stops
/// it afterwards.
#[test]
fn postgres_commands_use_the_development_database() {
	let Some(binaries) = postgres_binaries() else {
		return;
	};
	let root = tempfile::tempdir().unwrap();
	let migration = root.path().join("migrations/20261030T000000_init");
	fs::create_dir_all(&migration).unwrap();
	fs::write(
		migration.join("migration.sql"),
		"CREATE TABLE \"notes\" (\"id\" uuid PRIMARY KEY);\n",
	)
	.unwrap();
	let snapshot = leptoskit_db_schema::Schema::empty(leptoskit_db_schema::Dialect::Postgres);
	fs::write(migration.join("schema.json"), snapshot.to_json()).unwrap();

	fs::write(
		root.path().join("migrations/registry.txt"),
		"20261030T000000_init\n",
	)
	.unwrap();
	let config = "schema_version = 0\n\n[app]\nname = \"my-app\"\nversion = \"0.1.0\"\n\n[db]\n";
	fs::write(root.path().join("leptoskit.toml"), config).unwrap();
	let run = |args: &[&str]| {
		Command::new(env!("CARGO_BIN_EXE_leptoskit"))
			.args(args)
			.current_dir(root.path())
			.env_remove("DATABASE_URL")
			.env("LEPTOSKIT_POSTGRES_BIN", &binaries)
			.env("XDG_CACHE_HOME", root.path().join("cache"))
			.output()
			.expect("run leptoskit")
	};

	let migrated = succeeds(&run(&["db", "migrate"]));
	assert!(
		migrated.contains("using the development database postgres://postgres:***@localhost:"),
		"{migrated}"
	);
	assert!(migrated.contains("/my_app?"), "{migrated}");
	assert!(
		migrated.contains("applied 20261030T000000_init"),
		"{migrated}"
	);
	let data = root.path().join(".leptoskit/db/postgres/data");
	assert!(!data.join("postmaster.pid").exists(), "the server stopped");

	assert!(succeeds(&run(&["db", "migrate"])).contains("up to date"));
	assert!(succeeds(&run(&["db", "reset", "--yes"])).contains("applied 1"));
	assert!(!root.path().join("cache").exists());

	fs::write(
		root.path().join("leptoskit.toml"),
		format!("{config}dev_server = \"off\"\n"),
	)
	.unwrap();
	assert!(fails(&run(&["db", "migrate"])).contains("set DATABASE_URL"));
}
