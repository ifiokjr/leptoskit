//! A throwaway NATS server with JetStream, for the live relay's tests:
//! `LEPTOSKIT_NATS_SERVER`, or `nats-server` on the path (devenv has it).
//! Without one the tests that need it are skipped, unless
//! `LEPTOSKIT_REQUIRE_NATS` is set.

use std::env;
use std::net::TcpListener;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use tempfile::TempDir;

pub struct NatsServer {
	binary: PathBuf,
	store: TempDir,
	port: u16,
	child: Option<Child>,
}

impl NatsServer {
	/// Starts one on a free port, with JetStream storing into a temporary
	/// directory.
	pub fn start() -> Option<Self> {
		let Some(binary) = find() else {
			assert!(
				env::var_os("LEPTOSKIT_REQUIRE_NATS").is_none(),
				"LEPTOSKIT_REQUIRE_NATS is set, but no nats-server was found: run the tests in \
				 `devenv shell`, or set LEPTOSKIT_NATS_SERVER"
			);
			eprintln!(
				"skipping: no nats-server found (run in `devenv shell`, or set \
				 LEPTOSKIT_NATS_SERVER)"
			);
			return None;
		};

		let port = TcpListener::bind("127.0.0.1:0")
			.unwrap()
			.local_addr()
			.unwrap()
			.port();
		let mut server = Self {
			binary,
			store: tempfile::tempdir().unwrap(),
			port,
			child: None,
		};
		server.restart();
		Some(server)
	}

	/// Where clients connect.
	pub fn url(&self) -> String {
		format!("nats://127.0.0.1:{}", self.port)
	}

	/// Stops the server, as a crash would, keeping what JetStream stored.
	pub fn stop(&mut self) {
		if let Some(mut child) = self.child.take() {
			let _ = child.kill();
			let _ = child.wait();
		}
	}

	/// Starts it again on the same port and store, and returns once it
	/// accepts connections.
	pub fn restart(&mut self) {
		self.stop();
		let child = Command::new(&self.binary)
			.args(["-js", "-a", "127.0.0.1", "-p"])
			.arg(self.port.to_string())
			.arg("-sd")
			.arg(self.store.path())
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.expect("run nats-server");
		self.child = Some(child);
		let deadline = Instant::now() + Duration::from_secs(20);

		while TcpStream::connect(("127.0.0.1", self.port)).is_err() {
			assert!(Instant::now() < deadline, "nats-server didn't start");
			std::thread::sleep(Duration::from_millis(20));
		}
	}
}

impl Drop for NatsServer {
	fn drop(&mut self) {
		self.stop();
	}
}

fn find() -> Option<PathBuf> {
	if let Some(binary) = env::var_os("LEPTOSKIT_NATS_SERVER") {
		return Some(PathBuf::from(binary));
	}

	env::split_paths(&env::var_os("PATH")?)
		.map(|directory| directory.join("nats-server"))
		.find(|binary| binary.exists())
}
