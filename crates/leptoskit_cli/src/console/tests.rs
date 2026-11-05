//! Behavioural tests for the console entrypoints.
//!
//! Every test uses fresh temporary directories and, where it needs a service,
//! a scripted loopback HTTP server. Nothing reads the host's configuration,
//! credentials or Git state.

use std::path::PathBuf;

use leptoskit_deploy_console::control::Action;
use leptoskit_deploy_console::control::Intent;
use leptoskit_deploy_console::control::Project;
use leptoskit_deploy_console::control::Store;
use tempfile::TempDir;

use super::*;

/// A scripted HTTP server for the client modules' outbound requests.
pub(crate) mod loopback {
	use std::io::Read as _;
	use std::io::Write as _;
	use std::net::TcpListener;
	use std::net::TcpStream;
	use std::sync::Arc;
	use std::sync::Mutex;
	use std::sync::atomic::AtomicBool;
	use std::sync::atomic::Ordering;
	use std::time::Duration;
	use std::time::Instant;

	/// One deterministic reply, consumed in request order.
	pub(crate) struct Reply {
		status: u16,
		body: String,
		headers: Vec<(String, String)>,
	}

	impl Reply {
		pub(crate) fn json(body: impl Into<String>) -> Self {
			Self {
				status: 200,
				body: body.into(),
				headers: Vec::new(),
			}
		}

		pub(crate) fn status(status: u16, body: impl Into<String>) -> Self {
			Self {
				status,
				body: body.into(),
				headers: Vec::new(),
			}
		}

		pub(crate) fn redirect(location: &str) -> Self {
			Self {
				status: 302,
				body: String::new(),
				headers: vec![("Location".into(), location.into())],
			}
		}
	}

	/// One request the server accepted, parsed just far enough to assert
	/// exactly what the client sent.
	#[derive(Clone)]
	pub(crate) struct Request {
		pub(crate) method: String,
		pub(crate) path: String,
		pub(crate) headers: Vec<(String, String)>,
		pub(crate) body: String,
	}

	impl Request {
		pub(crate) fn header(&self, name: &str) -> Option<&str> {
			self.headers
				.iter()
				.find(|(header, _)| header == name)
				.map(|(_, value)| value.as_str())
		}

		pub(crate) fn json(&self) -> serde_json::Value {
			serde_json::from_str(&self.body).expect("request body should be JSON")
		}
	}

	/// A loopback origin plus the requests it recorded. Dropping it stops the
	/// accept loop, so no test leaves a server thread running.
	pub(crate) struct Server {
		origin: String,
		requests: Arc<Mutex<Vec<Request>>>,
		stop: Arc<AtomicBool>,
		thread: Option<std::thread::JoinHandle<()>>,
	}

	impl Server {
		pub(crate) fn start(replies: Vec<Reply>) -> Self {
			let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
			listener
				.set_nonblocking(true)
				.expect("nonblocking listener");
			let origin = format!("http://{}", listener.local_addr().unwrap());
			let requests = Arc::new(Mutex::new(Vec::new()));
			let stop = Arc::new(AtomicBool::new(false));
			let thread = {
				let requests = Arc::clone(&requests);
				let stop = Arc::clone(&stop);
				std::thread::spawn(move || {
					let deadline = Instant::now() + Duration::from_secs(30);
					serve(&listener, replies.into_iter(), &requests, &stop, deadline);
				})
			};
			Self {
				origin,
				requests,
				stop,
				thread: Some(thread),
			}
		}

		pub(crate) fn origin(&self) -> &str {
			&self.origin
		}

		pub(crate) fn requests(&self) -> Vec<Request> {
			self.requests.lock().unwrap().clone()
		}
	}

	impl Drop for Server {
		fn drop(&mut self) {
			self.stop.store(true, Ordering::SeqCst);
			if let Some(thread) = self.thread.take() {
				let _ = thread.join();
			}
		}
	}

	fn serve(
		listener: &TcpListener,
		mut replies: impl Iterator<Item = Reply>,
		requests: &Mutex<Vec<Request>>,
		stop: &AtomicBool,
		deadline: Instant,
	) {
		while !stop.load(Ordering::SeqCst) {
			let Some(reply) = replies.next() else {
				break;
			};
			let Some(mut stream) = accept(listener, stop, deadline) else {
				break;
			};
			stream.set_nonblocking(false).expect("blocking request");
			if let Some(request) = read_request(&mut stream) {
				requests.lock().unwrap().push(request);
			}
			write_reply(&mut stream, reply);
		}
	}

	fn accept(listener: &TcpListener, stop: &AtomicBool, deadline: Instant) -> Option<TcpStream> {
		loop {
			if stop.load(Ordering::SeqCst) || Instant::now() > deadline {
				return None;
			}
			match listener.accept() {
				Ok((stream, _)) => return Some(stream),
				Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
					std::thread::sleep(Duration::from_millis(2));
				}
				Err(_) => return None,
			}
		}
	}

	fn read_request(stream: &mut TcpStream) -> Option<Request> {
		stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
		let mut buffer = Vec::new();
		let mut chunk = [0_u8; 4096];
		let header_end = loop {
			let read = stream.read(&mut chunk).ok()?;
			if read == 0 {
				return None;
			}
			buffer.extend_from_slice(&chunk[..read]);
			if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
				break position;
			}
		};
		let head = String::from_utf8_lossy(&buffer[..header_end]);
		let mut lines = head.split("\r\n");
		let mut request = lines.next()?.split(' ');
		let method = request.next()?.to_owned();
		let path = request.next()?.to_owned();
		let headers = lines
			.filter_map(|line| line.split_once(':'))
			.map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
			.collect::<Vec<_>>();
		let expected = headers
			.iter()
			.find(|(name, _)| name == "content-length")
			.and_then(|(_, value)| value.parse::<usize>().ok())
			.unwrap_or_default();
		let mut body = buffer[header_end + 4..].to_vec();
		while body.len() < expected {
			let read = stream.read(&mut chunk).ok()?;
			if read == 0 {
				break;
			}
			body.extend_from_slice(&chunk[..read]);
		}
		Some(Request {
			method,
			path,
			headers,
			body: String::from_utf8_lossy(&body).into_owned(),
		})
	}

	fn write_reply(stream: &mut TcpStream, reply: Reply) {
		use std::fmt::Write as _;

		let reason = match reply.status {
			200 => "OK",
			204 => "No Content",
			302 => "Found",
			400 => "Bad Request",
			401 => "Unauthorized",
			403 => "Forbidden",
			409 => "Conflict",
			500 => "Internal Server Error",
			_ => "Status",
		};
		let mut head = format!(
			"HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: \
			 {}\r\nConnection: close\r\n",
			reply.status,
			reason,
			reply.body.len()
		);
		for (name, value) in reply.headers {
			let _ = write!(head, "{name}: {value}\r\n");
		}
		head.push_str("\r\n");
		let _ = stream.write_all(head.as_bytes());
		let _ = stream.write_all(reply.body.as_bytes());
		let _ = stream.flush();
	}
}

/// A temporary directory whose mode matches the private management
/// directory the production commands demand.
pub(crate) fn private_tempdir() -> TempDir {
	let directory = tempfile::tempdir().unwrap();
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
	}
	directory
}

/// A durable store with one registered repository and one queued operation.
struct Fixture {
	data: TempDir,
	out: TempDir,
}

impl Fixture {
	fn new() -> Self {
		let data = private_tempdir();
		let out = private_tempdir();
		let store = Store::open(&data.path().join("console.sqlite3")).unwrap();
		let project = Project {
			id: "demo".into(),
			environment: "production".into(),
			root: data.path().to_path_buf(),
			revision: 0,
		};
		store.register(&project).unwrap();
		store
			.enqueue(
				&project.id,
				&Intent {
					expected_revision: 0,
					idempotency_key: "queued-operation".into(),
					action: Action::Status,
				},
			)
			.unwrap();
		Self { data, out }
	}

	fn data(&self) -> PathBuf {
		self.data.path().to_path_buf()
	}

	fn database(&self) -> PathBuf {
		self.data.path().join("console.sqlite3")
	}

	fn store(&self) -> Store {
		Store::open(&self.database()).unwrap()
	}
}

#[test]
fn an_explicit_data_directory_is_used_verbatim() {
	let path = PathBuf::from("management/data");

	assert_eq!(data_directory(Some(path.clone())).unwrap(), path);
}

#[cfg(unix)]
#[test]
fn the_service_lock_is_private_and_exclusive() {
	use std::os::unix::fs::PermissionsExt as _;

	let root = tempfile::tempdir().unwrap();
	let path = root.path().join("data");
	let first = directory_lock(&path).unwrap();

	assert_eq!(
		std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
		0o700
	);
	assert!(path.join("service.lock").exists());
	let second = directory_lock(&path).unwrap_err();
	assert!(
		second
			.to_string()
			.contains("the deployment service is running"),
		"unexpected error: {second}"
	);

	drop(first);
	assert!(directory_lock(&path).is_ok());

	let public = root.path().join("public");
	std::fs::create_dir(&public).unwrap();
	std::fs::set_permissions(&public, std::fs::Permissions::from_mode(0o755)).unwrap();
	let error = directory_lock(&public).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("dedicated private management data directory"),
		"unexpected error: {error}"
	);
}

#[test]
fn backup_and_rollback_keep_recoverable_copies() {
	let fixture = Fixture::new();
	let backup = fixture.out.path().join("before-rollback.sqlite3");
	run(ConsoleCommand::Backup {
		data: Some(fixture.data()),
		out: backup.clone(),
	})
	.unwrap();
	assert_eq!(
		Store::open(&backup).unwrap().projects().unwrap().len(),
		1,
		"the backup must keep the registered project"
	);

	let rollback = fixture.out.path().join("rollback.sqlite3");
	let error = run(ConsoleCommand::RollbackDatabase {
		data: Some(fixture.data()),
		backup: rollback.clone(),
		yes: false,
	})
	.unwrap_err();
	assert!(
		error.to_string().contains("pass --yes"),
		"unexpected error: {error}"
	);
	assert!(!rollback.exists());

	run(ConsoleCommand::RollbackDatabase {
		data: Some(fixture.data()),
		backup: rollback.clone(),
		yes: true,
	})
	.unwrap();
	assert_eq!(
		Store::open(&rollback).unwrap().projects().unwrap().len(),
		1,
		"the rollback backup must stay recoverable"
	);
	let reopened = Store::open(&fixture.database()).unwrap();
	assert!(
		reopened.projects().unwrap().is_empty(),
		"rollback must delete the journal history"
	);
}

#[test]
fn backup_refuses_to_overwrite_an_existing_file() {
	let fixture = Fixture::new();
	let out = fixture.out.path().join("existing.sqlite3");
	std::fs::write(&out, "keep me").unwrap();

	let error = run(ConsoleCommand::Backup {
		data: Some(fixture.data()),
		out: out.clone(),
	})
	.unwrap_err();

	assert!(
		error.to_string().contains("backup already exists"),
		"unexpected error: {error}"
	);
	assert_eq!(std::fs::read(&out).unwrap(), b"keep me");
}

#[test]
fn restore_requires_confirmation_before_touching_the_data_directory() {
	let fixture = Fixture::new();
	let source = fixture.out.path().join("source.sqlite3");
	fixture
		.store()
		.backup(&source)
		.expect("write the source backup");
	let missing = fixture.out.path().join("missing-data");
	let previous = fixture.out.path().join("previous.sqlite3");

	let error = run(ConsoleCommand::Restore {
		data: Some(missing.clone()),
		from: source,
		previous: previous.clone(),
		yes: false,
	})
	.unwrap_err();

	assert!(
		error.to_string().contains("pass --yes"),
		"unexpected error: {error}"
	);
	assert!(!missing.exists());
	assert!(!previous.exists());
}

#[test]
fn restore_refuses_a_corrupt_backup_and_keeps_the_journal() {
	let fixture = Fixture::new();
	let source = fixture.out.path().join("corrupt.sqlite3");
	std::fs::write(&source, b"not a management database").unwrap();
	let previous = fixture.out.path().join("previous.sqlite3");

	let error = run(ConsoleCommand::Restore {
		data: Some(fixture.data()),
		from: source,
		previous: previous.clone(),
		yes: true,
	})
	.unwrap_err();

	assert!(
		error.to_string().contains("not a database"),
		"unexpected error: {error}"
	);
	assert!(!previous.exists());
	assert_eq!(
		Store::open(&fixture.database())
			.unwrap()
			.projects()
			.unwrap()
			.len(),
		1,
		"a rejected restore must leave the live journal untouched"
	);
}

#[test]
fn restore_keeps_the_previous_database_and_replaces_the_journal() {
	let fixture = Fixture::new();
	let source = fixture.out.path().join("source.sqlite3");
	fixture
		.store()
		.backup(&source)
		.expect("write the source backup");
	let store = Store::open(&fixture.database()).unwrap();
	store
		.register(&Project {
			id: "extra".into(),
			environment: "production".into(),
			root: fixture.data(),
			revision: 0,
		})
		.unwrap();
	let previous = fixture.out.path().join("previous.sqlite3");

	run(ConsoleCommand::Restore {
		data: Some(fixture.data()),
		from: source,
		previous: previous.clone(),
		yes: true,
	})
	.unwrap();

	let restored = Store::open(&fixture.database()).unwrap();
	let ids = restored
		.projects()
		.unwrap()
		.into_iter()
		.map(|project| project.id)
		.collect::<Vec<_>>();
	assert_eq!(ids, vec!["demo".to_owned()]);
	let saved = Store::open(&previous).unwrap();
	let saved_ids = saved
		.projects()
		.unwrap()
		.into_iter()
		.map(|project| project.id)
		.collect::<Vec<_>>();
	assert_eq!(saved_ids, vec!["demo".to_owned(), "extra".to_owned()]);
}

#[test]
fn restore_refuses_while_another_offline_command_holds_the_lock() {
	let fixture = Fixture::new();
	let source = fixture.out.path().join("source.sqlite3");
	Store::open(&fixture.database())
		.unwrap()
		.backup(&source)
		.unwrap();
	let previous = fixture.out.path().join("previous.sqlite3");
	let _lock = directory_lock(&fixture.data()).unwrap();

	let error = run(ConsoleCommand::Restore {
		data: Some(fixture.data()),
		from: source,
		previous: previous.clone(),
		yes: true,
	})
	.unwrap_err();

	assert!(
		error
			.to_string()
			.contains("the deployment service is running"),
		"unexpected error: {error}"
	);
	assert!(!previous.exists());
}

#[test]
fn restore_refuses_identical_paths() {
	let fixture = Fixture::new();
	let database = fixture.database();
	let previous = fixture.out.path().join("previous.sqlite3");

	let error = run(ConsoleCommand::Restore {
		data: Some(fixture.data()),
		from: database.clone(),
		previous: previous.clone(),
		yes: true,
	})
	.unwrap_err();

	assert!(
		error.to_string().contains("restore paths must be distinct"),
		"unexpected error: {error}"
	);
	assert!(!previous.exists());
}
