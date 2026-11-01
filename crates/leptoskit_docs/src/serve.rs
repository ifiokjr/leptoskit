//! A development server: serves a built site, rebuilds it when the book
//! changes, and reloads open pages after every rebuild.
//!
//! It speaks just enough HTTP/1.1 for a local browser, one connection per
//! request, so the crate needs no async runtime. It is for writing docs, not
//! for hosting them; publish the output of [`Site::build`] instead.
//!
//! File watchers drop events under load: macOS `FSEvents` and Linux `inotify`
//! both overflow, and say so only with a "rescan" event that names no file.
//! So the server treats rescans and watcher errors as changes, and also
//! compares the book's file sizes and modification times every
//! [`RESCAN`], so an edit no watcher reported still rebuilds the site.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use notify::RecursiveMode;
use notify::Watcher;

use crate::Error;
use crate::Site;
use crate::site::ASSETS;

/// How long a page's reload request waits for a rebuild before it asks again.
const POLL: Duration = Duration::from_secs(25);

/// How long a client may take to send its request.
const REQUEST: Duration = Duration::from_secs(10);

/// How long the watcher waits for a burst of file events to settle.
const SETTLE: Duration = Duration::from_millis(120);

/// How often the book is compared with the last build, for changes the
/// watcher did not report.
const RESCAN: Duration = Duration::from_secs(1);

/// A running [`Site::serve`].
pub struct Server {
	address: SocketAddr,
	thread: JoinHandle<()>,
}

impl Server {
	/// The address the server listens on, with the port the system chose when
	/// asked for port 0.
	pub fn address(&self) -> SocketAddr {
		self.address
	}

	/// Blocks for as long as the server runs, which is until the process
	/// exits.
	pub fn wait(self) {
		self.thread
			.join()
			.expect("the server thread does not panic");
	}
}

/// The build generation, bumped after every rebuild, that open pages wait on.
#[derive(Default)]
struct Generation {
	count: Mutex<u64>,
	changed: Condvar,
}

impl Generation {
	fn bump(&self) {
		*self.count.lock().expect("never poisoned") += 1;
		self.changed.notify_all();
	}

	/// The generation, once it differs from `seen` or the wait times out.
	fn after(&self, seen: u64) -> u64 {
		let count = self.count.lock().expect("never poisoned");
		let (count, _) = self
			.changed
			.wait_timeout_while(count, POLL, |count| *count == seen)
			.expect("never poisoned");
		*count
	}
}

impl Site {
	/// Builds the site into `out` and serves it on `address`, rebuilding
	/// whenever a file in the book changes. Open pages reload after each
	/// rebuild.
	///
	/// Build problems are printed rather than returned, so a mistake while
	/// writing leaves the server running with the last good build. Fails
	/// only when the address cannot be bound or the book cannot be watched.
	pub fn serve(
		mut self,
		out: impl Into<PathBuf>,
		address: impl ToSocketAddrs,
	) -> Result<Server, Error> {
		self.live_reload = true;
		let out = out.into();
		let listener = TcpListener::bind(address).map_err(Error::Listen)?;
		let address = listener.local_addr().map_err(Error::Listen)?;
		let generation = Arc::new(Generation::default());

		// Watchers report canonical paths, so the output is excluded by its
		// canonical path, which only exists once the directory does. A build
		// accepts an empty directory.
		fs::create_dir_all(&out).map_err(Error::io("create", &out))?;
		let canonical_out = fs::canonicalize(&out).map_err(Error::io("read", &out))?;
		let canonical_root = fs::canonicalize(&self.root).unwrap_or_else(|_| self.root.clone());
		// Taken before the first build, so an edit made while it runs, before
		// the watcher starts, is still found.
		let built = fingerprint(&canonical_root, &canonical_out);

		rebuild(&self, &out, &generation);

		let (sender, events) = mpsc::channel();
		let watch_error = |source| {
			Error::Watch {
				path: self.root.clone(),
				source,
			}
		};
		let mut watcher = notify::recommended_watcher(sender).map_err(watch_error)?;
		watcher
			.watch(&self.root, RecursiveMode::Recursive)
			.map_err(watch_error)?;

		let watched_out = out.clone();
		let watched_generation = Arc::clone(&generation);
		let mut changes = Changes {
			events,
			root: canonical_root,
			out: canonical_out,
			built,
			rescan: RESCAN,
		};
		thread::spawn(move || {
			// The watcher stops when dropped, so it lives on this thread.
			let _watcher = watcher;

			while changes.next() {
				rebuild(&self, &watched_out, &watched_generation);
			}
		});

		let thread = thread::spawn(move || {
			for stream in listener.incoming().flatten() {
				let out = out.clone();
				let generation = Arc::clone(&generation);
				thread::spawn(move || respond(&stream, &out, &generation));
			}
		});

		Ok(Server { address, thread })
	}
}

fn rebuild(site: &Site, out: &Path, generation: &Generation) {
	match site.build(out) {
		Ok(build) => {
			eprintln!(
				"built {} pages and copied {} files",
				build.pages, build.files
			);
		}
		Err(error) => eprintln!("{error}"),
	}

	generation.bump();
}

/// When the book changes: from the watcher's events, or from comparing the
/// book with the last build every `rescan`.
struct Changes {
	events: mpsc::Receiver<notify::Result<notify::Event>>,
	/// The book and the output inside it, both canonical.
	root: PathBuf,
	out: PathBuf,
	/// The [`fingerprint`] of the book when it was last built.
	built: u64,
	rescan: Duration,
}

impl Changes {
	/// Waits for a change to the book, then for the burst of events around it
	/// to settle. Returns `false` once the watcher has gone away.
	///
	/// Only changes to the book extend the wait. Serving pages makes the
	/// system report reads of the output, and a page that polls for reloads
	/// would keep a wait that counted every event from ever ending. For the
	/// same reason the comparison runs on a fixed schedule, however many
	/// unrelated events arrive.
	fn next(&mut self) -> bool {
		let mut compare = Instant::now() + self.rescan;

		loop {
			match self
				.events
				.recv_timeout(compare.saturating_duration_since(Instant::now()))
			{
				Ok(event) if reports_change(&event, &self.out) => break,
				Ok(_) => {}
				Err(mpsc::RecvTimeoutError::Timeout) => {
					if fingerprint(&self.root, &self.out) != self.built {
						break;
					}

					compare = Instant::now() + self.rescan;
				}

				Err(mpsc::RecvTimeoutError::Disconnected) => return false,
			}
		}

		let mut settled = Instant::now() + SETTLE;

		loop {
			match self
				.events
				.recv_timeout(settled.saturating_duration_since(Instant::now()))
			{
				Ok(event) if reports_change(&event, &self.out) => {
					settled = Instant::now() + SETTLE;
				}

				Ok(_) => {}
				Err(mpsc::RecvTimeoutError::Timeout) => break,

				Err(mpsc::RecvTimeoutError::Disconnected) => return false,
			}
		}

		// Taken before the rebuild, so an edit made while it runs is found
		// by the next comparison.
		self.built = fingerprint(&self.root, &self.out);
		true
	}
}

/// Whether an event may mean the book changed.
///
/// A watcher that dropped events reports a rescan, which names no file, and
/// a watcher error may hide a change too, so both count. Otherwise the event
/// must change something in the book rather than the output, which may live
/// inside it.
fn reports_change(event: &notify::Result<notify::Event>, out: &Path) -> bool {
	match event {
		Err(_) => true,

		Ok(event) if event.need_rescan() => true,
		Ok(event) => {
			!event.kind.is_access() && event.paths.iter().any(|path| !path.starts_with(out))
		}
	}
}

/// The path, size, and modification time of every file in the book, outside
/// the output and hidden files such as editor swap files, as one hash.
///
/// It reads metadata only, so it stays cheap for books full of images. A
/// file that cannot be read is left out, which changes the hash as well.
fn fingerprint(root: &Path, out: &Path) -> u64 {
	let mut hasher = DefaultHasher::new();
	let entries = walkdir::WalkDir::new(root)
		.sort_by_file_name()
		.into_iter()
		.filter_entry(|entry| {
			let hidden = entry.depth() > 0 && entry.file_name().to_string_lossy().starts_with('.');
			!hidden && !entry.path().starts_with(out)
		})
		.flatten();

	for entry in entries {
		let Ok(metadata) = entry.metadata() else {
			continue;
		};

		if metadata.is_file() {
			entry.path().hash(&mut hasher);
			metadata.len().hash(&mut hasher);
			metadata.modified().ok().hash(&mut hasher);
		}
	}

	hasher.finish()
}

fn respond(stream: &TcpStream, out: &Path, generation: &Generation) {
	// A client that never finishes its request must not hold a thread forever.
	if stream.set_read_timeout(Some(REQUEST)).is_err() {
		return;
	}

	let mut reader = BufReader::new(stream);
	let mut request = String::new();

	if reader.read_line(&mut request).is_err() {
		return;
	}

	let mut parts = request.split_whitespace();
	let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
		return reply(stream, "400 Bad Request", "text/plain", b"bad request");
	};

	// Drain the headers; nothing in them changes the response.
	let mut header = String::new();

	while reader.read_line(&mut header).is_ok_and(|read| read > 2) {
		header.clear();
	}

	if method != "GET" {
		return reply(stream, "405 Method Not Allowed", "text/plain", b"only GET");
	}

	let (path, query) = target.split_once('?').unwrap_or((target, ""));

	if path == format!("/{ASSETS}/live") {
		let seen = query
			.strip_prefix("since=")
			.and_then(|seen| seen.parse().ok())
			.unwrap_or(0);
		let count = generation.after(seen).to_string();

		return reply(stream, "200 OK", "text/plain", count.as_bytes());
	}

	let Some(file) = resolve(out, path) else {
		return reply(stream, "400 Bad Request", "text/plain", b"bad path");
	};

	if let Ok(body) = fs::read(&file) {
		reply(stream, "200 OK", content_type(&file), &body);
	} else {
		let body = fs::read(out.join("404.html")).unwrap_or_else(|_| b"not found".to_vec());
		reply(stream, "404 Not Found", "text/html; charset=utf-8", &body);
	}
}

/// The file in `out` for a request path, with `index.html` for directories,
/// or `None` for a path that climbs out of `out`.
fn resolve(out: &Path, path: &str) -> Option<PathBuf> {
	let decoded = decode(path)?;
	let relative = Path::new(decoded.trim_start_matches('/'));

	if relative
		.components()
		.any(|component| !matches!(component, Component::Normal(_)))
	{
		return None;
	}

	let file = out.join(relative);
	Some(if file.is_dir() {
		file.join("index.html")
	} else {
		file
	})
}

/// Percent-decodes a request path.
fn decode(path: &str) -> Option<String> {
	let bytes = path.as_bytes();
	let mut decoded = Vec::with_capacity(bytes.len());
	let mut index = 0;

	while index < bytes.len() {
		if bytes[index] == b'%' {
			let hex = path.get(index + 1..index + 3)?;
			decoded.push(u8::from_str_radix(hex, 16).ok()?);
			index += 3;
		} else {
			decoded.push(bytes[index]);
			index += 1;
		}
	}

	String::from_utf8(decoded).ok()
}

fn content_type(file: &Path) -> &'static str {
	match file
		.extension()
		.and_then(|extension| extension.to_str())
		.map(str::to_ascii_lowercase)
		.as_deref()
	{
		Some("html") => "text/html; charset=utf-8",
		Some("css") => "text/css; charset=utf-8",
		Some("js") => "text/javascript; charset=utf-8",
		Some("json") => "application/json",
		Some("svg") => "image/svg+xml",
		Some("png") => "image/png",

		Some("jpg" | "jpeg") => "image/jpeg",
		Some("gif") => "image/gif",
		Some("webp") => "image/webp",
		Some("ico") => "image/x-icon",
		Some("woff2") => "font/woff2",
		Some("txt") => "text/plain; charset=utf-8",
		_ => "application/octet-stream",
	}
}

fn reply(mut stream: &TcpStream, status: &str, content_type: &str, body: &[u8]) {
	let head = format!(
		"HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: \
		 {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
		body.len()
	);
	// The browser may have gone away; there is no one left to tell.
	let _ = stream
		.write_all(head.as_bytes())
		.and_then(|()| stream.write_all(body));
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn resolves_request_paths_inside_the_output() {
		let out = tempfile::tempdir().unwrap();
		fs::create_dir(out.path().join("guide")).unwrap();

		assert_eq!(
			resolve(out.path(), "/"),
			Some(out.path().join("index.html"))
		);
		assert_eq!(
			resolve(out.path(), "/guide"),
			Some(out.path().join("guide/index.html"))
		);
		assert_eq!(
			resolve(out.path(), "/a%20b.html"),
			Some(out.path().join("a b.html"))
		);
		assert_eq!(resolve(out.path(), "/../secret"), None);
		assert_eq!(resolve(out.path(), "/%2e%2e/secret"), None);
		assert_eq!(resolve(out.path(), "/bad%zz"), None);
		assert_eq!(resolve(out.path(), "/cut%2"), None);
	}

	#[test]
	fn names_content_types() {
		assert_eq!(
			content_type(Path::new("a.html")),
			"text/html; charset=utf-8"
		);
		assert_eq!(content_type(Path::new("a.svg")), "image/svg+xml");
		assert_eq!(content_type(Path::new("a.JPEG")), "image/jpeg");
		assert_eq!(
			content_type(Path::new("Makefile")),
			"application/octet-stream"
		);
	}

	fn event(kind: notify::EventKind, path: &Path) -> notify::Event {
		notify::Event::new(kind).add_path(path.to_owned())
	}

	fn output_event(kind: notify::EventKind, out: &Path) -> notify::Event {
		event(kind, &out.join("index.html"))
	}

	/// A [`Changes`] over a book in a fresh directory, with its output inside
	/// it, and the sender standing in for the watcher.
	fn changes(
		rescan: Duration,
	) -> (
		tempfile::TempDir,
		mpsc::Sender<notify::Result<notify::Event>>,
		Changes,
	) {
		let book = tempfile::tempdir().unwrap();
		let root = fs::canonicalize(book.path()).unwrap();
		let out = root.join("dist");
		fs::create_dir(&out).unwrap();
		fs::write(root.join("readme.md"), "# Home\n").unwrap();
		let (sender, events) = mpsc::channel();
		let built = fingerprint(&root, &out);

		(
			book,
			sender,
			Changes {
				events,
				root,
				out,
				built,
				rescan,
			},
		)
	}

	#[test]
	fn settles_on_book_changes_while_pages_keep_reading_the_output() {
		let (book, sender, mut changes) = changes(RESCAN);
		let out = changes.out.clone();
		let readme = changes.root.join("readme.md");
		let modify = notify::EventKind::Modify(notify::event::ModifyKind::Any);
		let read = notify::EventKind::Access(notify::event::AccessKind::Any);
		let reader = thread::spawn(move || {
			sender.send(Ok(output_event(modify, &out))).unwrap();
			sender.send(Ok(event(read, &readme))).unwrap();
			sender.send(Ok(event(modify, &readme))).unwrap();
			// A page polling the output must not hold the rebuild back.
			for _ in 0..40 {
				sender.send(Ok(output_event(modify, &out))).unwrap();
				thread::sleep(Duration::from_millis(10));
			}

			sender
		});

		let started = Instant::now();
		assert!(changes.next());
		assert!(
			started.elapsed() < Duration::from_millis(350),
			"{:?}",
			started.elapsed()
		);

		drop(reader.join().unwrap());
		assert!(!changes.next(), "the watcher went away");
		drop(book);
	}

	#[test]
	fn counts_rescans_and_watcher_errors_as_changes() {
		let (_book, sender, mut changes) = changes(Duration::from_secs(60));
		let rescan =
			notify::Event::new(notify::EventKind::Other).set_flag(notify::event::Flag::Rescan);

		sender.send(Ok(rescan)).unwrap();
		assert!(
			changes.next(),
			"a rescan names no file but may hide any edit"
		);

		sender
			.send(Err(notify::Error::generic("queue overflow")))
			.unwrap();
		assert!(changes.next(), "a watcher error may hide an edit");

		drop(sender);
		assert!(!changes.next());
	}

	#[test]
	fn finds_edits_the_watcher_never_reported() {
		let (_book, sender, mut changes) = changes(Duration::from_millis(40));
		let readme = changes.root.join("readme.md");
		let modify = notify::EventKind::Modify(notify::event::ModifyKind::Any);
		let out = changes.out.clone();

		// A watcher that went silent, while the page keeps polling the output.
		let noise = thread::spawn(move || {
			for _ in 0..30 {
				sender.send(Ok(output_event(modify, &out))).unwrap();
				thread::sleep(Duration::from_millis(5));
			}

			sender
		});
		fs::write(&readme, "# Home\n\nAn edit no event reported.\n").unwrap();

		let started = Instant::now();
		assert!(changes.next());
		assert!(
			started.elapsed() < Duration::from_secs(2),
			"{:?}",
			started.elapsed()
		);
		let sender = noise.join().unwrap();

		// Writing the output, as a rebuild does, is not a change to the book,
		// and neither are hidden files such as an editor's swap file.
		fs::write(changes.out.join("index.html"), "<html></html>").unwrap();
		fs::write(changes.root.join(".readme.md.swp"), "swap").unwrap();
		let quiet = thread::spawn(move || {
			thread::sleep(Duration::from_millis(200));
			drop(sender);
		});
		assert!(!changes.next(), "nothing in the book changed");
		quiet.join().unwrap();
	}

	#[test]
	fn stops_waiting_when_the_watcher_goes_away_mid_burst() {
		let (_book, sender, mut changes) = changes(RESCAN);
		let readme = changes.root.join("readme.md");
		sender
			.send(Ok(event(
				notify::EventKind::Create(notify::event::CreateKind::File),
				&readme,
			)))
			.unwrap();
		drop(sender);

		assert!(!changes.next());
	}

	#[test]
	fn fingerprints_the_book_but_not_its_output() {
		let (_book, _sender, changes) = changes(RESCAN);
		let before = fingerprint(&changes.root, &changes.out);

		fs::write(changes.out.join("page.html"), "built").unwrap();
		assert_eq!(fingerprint(&changes.root, &changes.out), before);

		fs::write(changes.root.join("guide.md"), "# Guide\n").unwrap();
		assert_ne!(fingerprint(&changes.root, &changes.out), before);
	}

	#[test]
	fn wakes_waiters_on_a_new_generation() {
		let generation = Arc::new(Generation::default());
		let waiter = {
			let generation = Arc::clone(&generation);
			thread::spawn(move || generation.after(0))
		};

		thread::sleep(Duration::from_millis(20));
		generation.bump();

		assert_eq!(waiter.join().unwrap(), 1);
		assert_eq!(generation.after(0), 1, "an old generation returns at once");
	}
}
