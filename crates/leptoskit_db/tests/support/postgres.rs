//! A throwaway Postgres cluster for tests, found the same way as in
//! `leptoskit_db_schema`'s apply tests: `LEPTOSKIT_POSTGRES_BIN`, `PGBIN`, the
//! path, or a Debian-style install. Without binaries the tests that need it
//! are skipped, unless `LEPTOSKIT_REQUIRE_POSTGRES` is set, as it is in CI.

use std::env;
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use tempfile::TempDir;

pub struct PostgresCluster {
	binaries: PathBuf,
	directory: TempDir,
	port: u16,
	databases: AtomicUsize,
}

impl PostgresCluster {
	pub fn start() -> Option<Self> {
		let Some((binaries, server)) = find_binaries() else {
			assert!(
				env::var_os("LEPTOSKIT_REQUIRE_POSTGRES").is_none(),
				"LEPTOSKIT_REQUIRE_POSTGRES is set, but no Postgres binaries were found"
			);
			eprintln!("skipping: no Postgres binaries (initdb, pg_ctl, psql) found");

			return None;
		};

		let directory = tempfile::tempdir().unwrap();
		let data = directory.path().join("data");
		let port = TcpListener::bind("127.0.0.1:0")
			.unwrap()
			.local_addr()
			.unwrap()
			.port();

		run(Command::new(binaries.join("initdb"))
			.arg("-D")
			.arg(&data)
			.args([
				"-U",
				"postgres",
				"-A",
				"trust",
				"-E",
				"UTF8",
				"--locale=C",
				"--no-sync",
			]));

		let socket_option = if cfg!(windows) {
			""
		} else {
			" -c unix_socket_directories=''"
		};

		// No pipes: on Windows the server inherits pg_ctl's output handles, so
		// waiting for them to close would wait for the server to exit. Its
		// output goes to the log instead.
		let log = directory.path().join("log");
		let started = Command::new(binaries.join("pg_ctl"))
			.arg("-D")
			.arg(&data)
			// The `postgres` beside where the binaries were found: a Nix
			// `postgresql.withPackages` environment links `initdb` to plain
			// Postgres but builds its own `postgres`, which finds the
			// extensions.
			.arg("-p")
			.arg(&server)
			.arg("-l")
			.arg(&log)
			.arg("-o")
			.arg(format!(
				"-p {port} -c listen_addresses=127.0.0.1 -F{socket_option}"
			))
			.args(["-w", "start"])
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.status()
			.expect("run pg_ctl");
		assert!(
			started.success(),
			"pg_ctl couldn't start Postgres:\n{}",
			fs::read_to_string(&log).unwrap_or_default()
		);

		Some(Self {
			binaries,
			directory,
			port,
			databases: AtomicUsize::new(0),
		})
	}

	/// A cluster that has `extension` to install, such as `postgis`. Without
	/// it the tests that need it are skipped, unless
	/// `LEPTOSKIT_REQUIRE_POSTGRES_EXTENSIONS` names it, as it does in CI.
	#[allow(
		dead_code,
		reason = "only the geospatial and vector tests need extensions"
	)]
	pub fn start_with(extension: &str) -> Option<Self> {
		let cluster = Self::start()?;
		let available = run(Command::new(cluster.binaries.join("psql"))
			.args([
				"-X",
				"-q",
				"-A",
				"-t",
				"-h",
				"127.0.0.1",
				"-U",
				"postgres",
				"-d",
				"postgres",
			])
			.arg("-p")
			.arg(cluster.port.to_string())
			.arg("-c")
			.arg(format!(
				"SELECT count(*) FROM pg_available_extensions WHERE name = '{extension}'"
			)));

		if available.trim() == "1" {
			return Some(cluster);
		}

		let required = env::var("LEPTOSKIT_REQUIRE_POSTGRES_EXTENSIONS").unwrap_or_default();
		assert!(
			!required.split(',').any(|name| name.trim() == extension),
			"LEPTOSKIT_REQUIRE_POSTGRES_EXTENSIONS names `{extension}`, but this Postgres can't \
			 install it"
		);
		eprintln!("skipping: this Postgres has no `{extension}` extension");
		None
	}

	/// A new, empty database, as a connection URL.
	pub fn database(&self) -> String {
		let name = format!("test_{}", self.databases.fetch_add(1, Ordering::Relaxed));
		run(Command::new(self.binaries.join("psql"))
			.args([
				"-X",
				"-q",
				"-v",
				"ON_ERROR_STOP=1",
				"-h",
				"127.0.0.1",
				"-U",
				"postgres",
				"-d",
				"postgres",
			])
			.arg("-p")
			.arg(self.port.to_string())
			.arg("-c")
			.arg(format!("CREATE DATABASE {name}")));

		format!(
			"postgres://postgres@127.0.0.1:{}/{name}?sslmode=disable",
			self.port
		)
	}
}

impl Drop for PostgresCluster {
	fn drop(&mut self) {
		let _ = Command::new(self.binaries.join("pg_ctl"))
			.arg("-D")
			.arg(self.directory.path().join("data"))
			.args(["-m", "immediate", "-w", "stop"])
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.status();
	}
}

fn find_binaries() -> Option<(PathBuf, PathBuf)> {
	let has_initdb = |directory: &Path| {
		directory.join("initdb").exists() || directory.join("initdb.exe").exists()
	};
	let from_env = ["LEPTOSKIT_POSTGRES_BIN", "PGBIN"]
		.into_iter()
		.filter_map(env::var_os)
		.map(PathBuf::from);
	let from_path = env::var_os("PATH")
		.into_iter()
		.flat_map(|path| env::split_paths(&path).collect::<Vec<_>>());
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

	let directory = from_env
		.chain(from_path)
		.chain(debian)
		.find(|directory| has_initdb(directory))?;

	// On Unix, Nix profiles and Homebrew link the binaries, and initdb finds
	// its data files relative to where it really is. Windows needs no
	// resolving, and canonical paths there take a form initdb can't use.
	if cfg!(windows) {
		return Some((directory.clone(), directory.join("postgres.exe")));
	}

	let server = directory.join("postgres");
	let binaries = fs::canonicalize(directory.join("initdb"))
		.ok()?
		.parent()
		.map(Path::to_path_buf)?;
	Some((binaries, server))
}

fn run(command: &mut Command) -> String {
	let output = command
		.output()
		.unwrap_or_else(|error| panic!("couldn't run {command:?}: {error}"));
	assert!(
		output.status.success(),
		"{command:?} failed:\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
	String::from_utf8(output.stdout).unwrap()
}
