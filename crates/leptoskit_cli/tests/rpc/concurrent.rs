//! Two actual CLI exports sharing Cargo's target, with explicit server gates.

use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::net::TcpListener;
use std::net::TcpStream;
use std::process::Child;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use super::*;

const GATE_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const GATED_SERVER: &str = r#"
use std::io::{Read, Write};
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

let out = std::env::var_os("LEPTOSKIT_RPC_SPEC_OUT").expect("export path");

if std::env::var_os("WRITE_SPEC").is_some() {
    std::fs::copy("spec.json", &out).expect("write spec");
}

let gate = std::env::var("EXPORT_GATE").expect("gate");
let mut stream = std::net::TcpStream::connect(gate).expect("connect gate");
stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).unwrap();
writeln!(stream, "{}", std::path::Path::new(&out).display()).unwrap();
stream.read_exact(&mut [0]).expect("release gate");
"#;

struct Export {
	child: Option<Child>,
}

impl Export {
	fn start(root: &Path, target: &Path, gate: &TcpListener, writes: bool) -> Self {
		let mut command = Command::new(env!("CARGO_BIN_EXE_leptoskit"));
		command
			.args(["rpc", "spec"])
			.current_dir(root)
			.env("CARGO_TARGET_DIR", target)
			// The dependency-free fixtures must not spend their gate deadline
			// waiting for unrelated builds to unlock the global package cache.
			.env("CARGO_HOME", target.join("cargo-home"))
			.env("CARGO_NET_OFFLINE", "true")
			.env("LEPTOSKIT_RPC_EXPORT_TIMEOUT_SECS", "30")
			.env(
				"EXPORT_GATE",
				gate.local_addr().expect("gate address").to_string(),
			)
			.env_remove("WRITE_SPEC")
			.env("CARGO_TERM_VERBOSE", "true")
			.stdout(Stdio::piped())
			.stderr(Stdio::piped());

		if writes {
			command.env("WRITE_SPEC", "1");
		}

		Self {
			child: Some(command.spawn().expect("start CLI export")),
		}
	}

	fn finish(mut self) -> Output {
		self.child
			.take()
			.expect("owned child")
			.wait_with_output()
			.expect("join CLI export")
	}
}

impl Drop for Export {
	fn drop(&mut self) {
		if let Some(mut child) = self.child.take() {
			let _ = child.kill();
			let _ = child.wait();
		}
	}
}

fn gate() -> TcpListener {
	let listener = TcpListener::bind("127.0.0.1:0").expect("bind export gate");
	listener.set_nonblocking(true).expect("nonblocking gate");
	listener
}

fn await_server(listener: &TcpListener) -> (TcpStream, PathBuf) {
	let started = Instant::now();

	loop {
		match listener.accept() {
			Ok((stream, _)) => {
				stream
					.set_nonblocking(false)
					.expect("blocking export handshake");
				stream
					.set_read_timeout(Some(GATE_TIMEOUT))
					.expect("gate deadline");
				let mut path = String::new();
				BufReader::new(&stream)
					.read_line(&mut path)
					.expect("export path");
				return (stream, PathBuf::from(path.trim_end()));
			}

			Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
			Err(error) => panic!("accept export gate: {error}"),
		}

		assert!(
			started.elapsed() < GATE_TIMEOUT,
			"server did not reach its export gate"
		);
		std::thread::sleep(POLL_INTERVAL);
	}
}

#[test]
fn concurrent_exports_cannot_supply_a_spec_to_a_nonexporting_server() {
	let scratch = Scratch::new("concurrent-export");
	let negative = scratch.path.join("negative");
	let positive = scratch.path.join("positive");
	let target = scratch.path.join("shared-target");

	for (root, name) in [
		(&negative, "isolated-negative"),
		(&positive, "isolated-positive"),
	] {
		std::fs::create_dir_all(root).expect("project directory");
		fake_server(root, name, GATED_SERVER);
		std::fs::write(root.join("spec.json"), SPEC).expect("fixture spec");
		// Compile both fixtures before the gates start timing the running
		// servers.
		let built = Command::new("cargo")
			.args(["build", "--package", name, "--quiet"])
			.current_dir(root)
			.env("CARGO_TARGET_DIR", &target)
			.env("CARGO_HOME", target.join("cargo-home"))
			.env("CARGO_NET_OFFLINE", "true")
			.output()
			.expect("build export fixture");
		assert!(built.status.success(), "{}", stderr(&built));
	}

	let negative_gate = gate();
	let negative_export = Export::start(&negative, &target, &negative_gate, false);
	let (mut negative_stream, negative_path) = await_server(&negative_gate);
	let positive_gate = gate();
	let positive_export = Export::start(&positive, &target, &positive_gate, true);
	let (mut positive_stream, positive_path) = await_server(&positive_gate);
	assert!(
		positive_path.is_file(),
		"positive server has written its export"
	);
	negative_stream
		.write_all(&[1])
		.expect("release negative server");

	let rejected = negative_export.finish();
	assert!(
		positive_path.is_file(),
		"negative CLI must leave the positive export alive"
	);
	positive_stream
		.write_all(&[1])
		.expect("release positive server");
	let accepted = positive_export.finish();
	retain_evidence(&scratch.path, &rejected, &accepted);
	assert_exports(&rejected, &accepted, &negative_path, &positive_path);
}

fn assert_exports(
	rejected: &Output,
	accepted: &Output,
	negative_path: &Path,
	positive_path: &Path,
) {
	assert!(
		!rejected.status.success(),
		"nonexporting server consumed another export: {}",
		stdout(rejected)
	);
	assert!(
		stderr(rejected).contains("wrote no spec"),
		"{}",
		stderr(rejected)
	);
	assert!(accepted.status.success(), "{}", stderr(accepted));
	let spec: serde_json::Value = serde_json::from_str(&stdout(accepted)).expect("exported JSON");
	assert_eq!(spec["title"], "Todos");
	assert_ne!(
		negative_path, positive_path,
		"each invocation owns its output"
	);
	assert!(!positive_path.parent().expect("output directory").exists());
	assert!(!negative_path.parent().expect("output directory").exists());
}

/// Retains actual child sources, producer logs and binaries for explicit local
/// verification.
fn retain_evidence(root: &Path, rejected: &Output, accepted: &Output) {
	let Some(destination) = std::env::var_os("LEPTOSKIT_EXPORT_TEST_EVIDENCE") else {
		return;
	};
	let destination = PathBuf::from(destination);
	std::fs::create_dir(&destination).expect("fresh export evidence directory");

	for (label, binary, output) in [
		("negative", "isolated-negative", rejected),
		("positive", "isolated-positive", accepted),
	] {
		let directory = destination.join(label);
		std::fs::create_dir(&directory).expect("child evidence directory");

		for file in [
			"Cargo.toml",
			"Cargo.lock",
			"leptoskit.toml",
			"src/main.rs",
			"spec.json",
		] {
			let target = directory.join(file.replace('/', "-"));
			std::fs::copy(root.join(label).join(file), target).expect("retain child source");
		}

		let executable = format!("{binary}{}", std::env::consts::EXE_SUFFIX);
		std::fs::copy(
			root.join("shared-target/debug").join(executable),
			directory.join("server.executable"),
		)
		.expect("retain actual server executable");
		std::fs::write(directory.join("cli.stdout"), &output.stdout).expect("retain stdout");
		std::fs::write(directory.join("cli.stderr"), &output.stderr).expect("retain stderr");
	}
}
