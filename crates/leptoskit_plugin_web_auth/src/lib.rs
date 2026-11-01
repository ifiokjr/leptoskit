//! Sign-in sessions in the system browser, never a web view inside the app,
//! as RFC 8252 (OAuth 2.0 for native apps) asks.
//!
//! A session has two steps. [`BEGIN`] says where the browser will come back
//! to the app: a private-use scheme named after the app's ID on iOS, macOS,
//! and Android, or a loopback address on Windows and Linux. The app gives
//! that address to its server when it starts the sign-in. [`AUTHENTICATE`]
//! then opens the provider's page and waits until the browser comes back
//! there, answering with the address it came back to.
//!
//! | Platform      | Browser                        | Comes back through                       |
//! | ------------- | ------------------------------ | ---------------------------------------- |
//! | iOS, macOS    | `ASWebAuthenticationSession`   | its callback, only to this session       |
//! | Android       | Auth Tab (Chrome 137 or later) | its activity result, only to this app    |
//! | Windows/Linux | the default browser            | `http://127.0.0.1:<port>`, bound for it  |
//!
//! Nothing that comes back is a credential on its own: the server finishes
//! the sign-in only together with a binding the app kept in memory.

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

/// Stable plugin registration name.
pub const NAME: &str = "web-auth";
/// Empty payload; answers a [`Session`].
pub const BEGIN: MethodId = MethodId(0);
/// [`encode_authenticate`] payload; answers the UTF-8 address the browser
/// came back to.
pub const AUTHENTICATE: MethodId = MethodId(1);
/// Exported methods, independently grantable by the host.
pub const METHODS: &[MethodId] = &[BEGIN, AUTHENTICATE];
/// The path every return address has.
pub const RETURN_PATH: &str = "/auth/native/return";
/// The longest address accepted either way.
pub const MAX_URL_BYTES: usize = 8192;

/// A session that has begun: its ID, and where the browser will come back
/// to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
	/// Names the session in [`AUTHENTICATE`].
	pub id: u32,
	/// The return address to give the server.
	pub return_to: String,
}

impl Session {
	/// The four-byte little-endian ID, then the address.
	#[must_use]
	pub fn encode(&self) -> Vec<u8> {
		let mut bytes = self.id.to_le_bytes().to_vec();
		bytes.extend_from_slice(self.return_to.as_bytes());
		bytes
	}

	/// Reads a [`BEGIN`] answer, checking the address is one a session can
	/// come back to.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let (id, rest) = split_id(payload)?;
		let return_to = utf8(rest)?;
		return_address(return_to)?;
		Ok(Self {
			id,
			return_to: return_to.to_owned(),
		})
	}
}

/// The [`AUTHENTICATE`] payload: the session's ID, then the page to open.
#[must_use]
pub fn encode_authenticate(session: u32, url: &str) -> Vec<u8> {
	let mut bytes = session.to_le_bytes().to_vec();
	bytes.extend_from_slice(url.as_bytes());
	bytes
}

/// Reads an [`AUTHENTICATE`] payload, checking the page is one to open.
pub fn decode_authenticate(payload: &[u8]) -> Result<(u32, url::Url), Error> {
	let (id, rest) = split_id(payload)?;
	Ok((id, authorization_url(utf8(rest)?)?))
}

fn split_id(payload: &[u8]) -> Result<(u32, &[u8]), Error> {
	let (id, rest) = payload
		.split_first_chunk::<4>()
		.ok_or(Error::InvalidArgument)?;
	Ok((u32::from_le_bytes(*id), rest))
}

fn utf8(bytes: &[u8]) -> Result<&str, Error> {
	if bytes.is_empty() || bytes.len() > MAX_URL_BYTES {
		return Err(Error::InvalidArgument);
	}

	let text = core::str::from_utf8(bytes).map_err(|_| Error::InvalidArgument)?;

	if text.bytes().any(|byte| !byte.is_ascii_graphic()) {
		return Err(Error::InvalidArgument);
	}

	Ok(text)
}

/// A provider's sign-in page: `https`, or `http` on this device for
/// development, with no credentials in it.
pub fn authorization_url(value: &str) -> Result<url::Url, Error> {
	let url = url::Url::parse(utf8(value.as_bytes())?).map_err(|_| Error::InvalidArgument)?;
	let local = matches!(url.host(), Some(url::Host::Domain("localhost")))
		|| matches!(url.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback())
		|| matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback());
	let scheme = url.scheme() == "https" || (url.scheme() == "http" && local);

	if !scheme || url.host().is_none() || !url.username().is_empty() || url.password().is_some() {
		return Err(Error::InvalidArgument);
	}

	Ok(url)
}

/// Checks a return address: `<scheme>:/auth/native/return` for a
/// reverse-DNS private-use scheme, `http://127.0.0.1:<port>` or
/// `http://[::1]:<port>` with that path, or `https://<host>` with it.
pub fn return_address(value: &str) -> Result<url::Url, Error> {
	let url = url::Url::parse(utf8(value.as_bytes())?).map_err(|_| Error::InvalidArgument)?;
	let clean = url.path() == RETURN_PATH
		&& url.query().is_none()
		&& url.fragment().is_none()
		&& url.username().is_empty()
		&& url.password().is_none();

	let shape = match url.scheme() {
		"https" => url.host().is_some(),
		"http" => {
			url.port().is_some()
				&& (matches!(url.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback())
					|| matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback()))
		}
		scheme => {
			reverse_dns(scheme)
				&& url.host().is_none()
				&& value == format!("{scheme}:{RETURN_PATH}")
		}
	};

	if clean && shape {
		Ok(url)
	} else {
		Err(Error::InvalidArgument)
	}
}

/// The private-use return address for the app `id`, such as
/// `com.example.app:/auth/native/return` for `com.example.App`.
pub fn scheme_return(id: &str) -> Result<String, Error> {
	let scheme = id.to_ascii_lowercase();

	if !reverse_dns(&scheme) {
		return Err(Error::InvalidArgument);
	}

	Ok(format!("{scheme}:{RETURN_PATH}"))
}

/// At least two dot-separated segments of lowercase ASCII letters and
/// digits, each starting with a letter, as RFC 8252 §7.1 asks.
fn reverse_dns(scheme: &str) -> bool {
	let segments: Vec<&str> = scheme.split('.').collect();
	segments.len() >= 2
		&& segments.iter().all(|segment| {
			segment.starts_with(|first: char| first.is_ascii_lowercase())
				&& segment
					.bytes()
					.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
		})
}

/// Checks the address the browser came back to is the session's own
/// return address, with only a query added, and answers it.
pub fn came_back(return_to: &str, address: &str) -> Result<String, Error> {
	let expected = return_address(return_to)?;
	let back = url::Url::parse(utf8(address.as_bytes())?).map_err(|_| Error::InvalidArgument)?;
	let same = back.scheme() == expected.scheme()
		&& back.host() == expected.host()
		&& back.port() == expected.port()
		&& back.path() == expected.path()
		&& back.username().is_empty()
		&& back.password().is_none()
		&& back.fragment().is_none();

	if same {
		Ok(address.to_owned())
	} else {
		Err(Error::InvalidArgument)
	}
}

/// The system browser on Windows, Linux, and macOS without a host app:
/// a loopback listener on a port of its own, for one session at a time.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub mod desktop {
	use std::io::Read;
	use std::io::Write;
	use std::net::Ipv4Addr;
	use std::net::TcpListener;
	use std::net::TcpStream;
	use std::time::Duration;
	use std::time::Instant;

	use leptoskit_bridge::Bytes;
	use leptoskit_bridge::Plugin;

	use super::*;

	/// How long a person has to sign in: the server's flow lasts as long.
	pub const TIMEOUT: Duration = Duration::from_mins(10);

	/// How long a waiting listener sleeps between polls.
	const POLL_INTERVAL: Duration = Duration::from_millis(25);

	/// How long one return request may take to arrive.
	const READ_TIMEOUT: Duration = Duration::from_secs(5);

	/// How much of a return request is read at a time.
	const CHUNK_BYTES: usize = 1024;

	/// The longest request the listener reads.
	const MAX_REQUEST: usize = 8192;

	/// What the browser shows once it has come back.
	const PAGE: &str = "<!doctype html><meta charset=utf-8><title>Signed in</title><p>You can \
	                    close this tab and go back to the app.";

	/// A loopback listener for one session.
	#[derive(Debug)]
	pub struct Loopback {
		listener: TcpListener,
		return_to: String,
	}

	impl Loopback {
		/// Listens on `127.0.0.1`, on a port the system picks.
		pub fn bind() -> Result<Self, Error> {
			let listener =
				TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|_| Error::Native)?;
			let port = listener.local_addr().map_err(|_| Error::Native)?.port();
			listener.set_nonblocking(true).map_err(|_| Error::Native)?;
			Ok(Self {
				listener,
				return_to: format!("http://127.0.0.1:{port}{RETURN_PATH}"),
			})
		}

		/// Where the browser comes back to.
		#[must_use]
		pub fn return_to(&self) -> &str {
			&self.return_to
		}

		/// Waits up to `timeout` for the browser to come back, and answers
		/// the address it came back to. Requests for other paths, such as
		/// `/favicon.ico`, get `404` and the wait goes on.
		pub fn wait(&self, timeout: Duration) -> Result<String, Error> {
			let deadline = Instant::now() + timeout;

			loop {
				match self.listener.accept() {
					Ok((stream, _)) => {
						if let Some(address) = self.answer(stream) {
							return Ok(address);
						}
					}

					Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
						if Instant::now() >= deadline {
							return Err(Error::Cancelled);
						}

						std::thread::sleep(POLL_INTERVAL);
					}

					Err(_) => return Err(Error::Native),
				}
			}
		}

		/// Reads one request; the address it came back to when it's the
		/// return path.
		fn answer(&self, mut stream: TcpStream) -> Option<String> {
			let _ = stream.set_nonblocking(false);
			let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
			let mut request = Vec::new();
			let mut chunk = [0; CHUNK_BYTES];

			while !request.windows(2).any(|pair| pair == b"\r\n") && request.len() < MAX_REQUEST {
				match stream.read(&mut chunk) {
					Ok(0) | Err(_) => break,
					Ok(read) => request.extend_from_slice(&chunk[..read]),
				}
			}

			let line = request.split(|byte| *byte == b'\r').next()?;
			let line = core::str::from_utf8(line).ok()?;
			let mut parts = line.split(' ');

			let target = match (parts.next(), parts.next(), parts.next()) {
				(Some("GET"), Some(target), Some(version)) if version.starts_with("HTTP/1.") => {
					target
				}

				_ => {
					respond(&mut stream, "400 Bad Request", "");
					return None;
				}
			};

			let origin = self.return_to.strip_suffix(RETURN_PATH)?;
			let address = format!("{origin}{target}");

			if let Ok(address) = came_back(&self.return_to, &address) {
				respond(&mut stream, "200 OK", PAGE);
				Some(address)
			} else {
				respond(&mut stream, "404 Not Found", "");
				None
			}
		}
	}

	fn respond(stream: &mut TcpStream, status: &str, body: &str) {
		let response = format!(
			"HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: \
			 {}\r\nCache-Control: no-store\r\nReferrer-Policy: \
			 no-referrer\r\nContent-Security-Policy: default-src 'none'\r\nConnection: \
			 close\r\n\r\n{body}",
			body.len()
		);
		let _ = stream.write_all(response.as_bytes());
		let _ = stream.flush();
	}

	/// Opens `url` in the default browser and waits for it to come back to
	/// `loopback`. The URL is passed to the launcher as one argument, never
	/// as shell text.
	pub fn authenticate(
		loopback: &Loopback,
		url: &url::Url,
		timeout: Duration,
	) -> Result<String, Error> {
		open::that(url.as_str()).map_err(|_| Error::Unavailable)?;
		loopback.wait(timeout)
	}

	/// The plugin, for a host that registers Rust plugins itself. Waiting
	/// for the browser blocks, for as long as the person takes to sign in,
	/// so run it on a worker, never the render thread.
	#[derive(Debug, Default)]
	pub struct WebAuth {
		session: Option<(u32, Loopback)>,
		next: u32,
	}

	impl Plugin for WebAuth {
		fn name(&self) -> &'static str {
			NAME
		}

		fn methods(&self) -> &'static [MethodId] {
			METHODS
		}

		fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
			match method {
				BEGIN if payload.is_empty() => {
					let loopback = Loopback::bind()?;
					self.next = self.next.wrapping_add(1);
					let session = Session {
						id: self.next,
						return_to: loopback.return_to().to_owned(),
					};
					// A newer session replaces one that never opened.
					self.session = Some((session.id, loopback));
					Ok(Bytes::from(session.encode()))
				}

				AUTHENTICATE => {
					let (id, url) = decode_authenticate(&payload)?;
					let (session, loopback) = self.session.take().ok_or(Error::InvalidCall)?;

					if session != id {
						return Err(Error::InvalidCall);
					}

					authenticate(&loopback, &url, TIMEOUT).map(Bytes::from)
				}
				BEGIN => Err(Error::InvalidArgument),
				_ => Err(Error::Unavailable),
			}
		}
	}
}

/// Cached native bridge client for an interpreted Wasm guest.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Resolves the plugin once and reuses its numeric handle.
	#[derive(Debug, Clone, Copy)]
	pub struct WebAuth(leptoskit_bridge::guest::Client);

	impl WebAuth {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Begins a session: where the browser will come back to.
		pub async fn begin(self) -> Result<Session, Error> {
			Session::decode(&self.0.call(BEGIN, &[])?.await?)
		}

		/// Opens `url` in the system browser, and waits for it to come back
		/// to the session's return address. Dropping the future closes the
		/// browser session. Fails with `Cancelled` when the person closes it.
		pub async fn authenticate(self, session: &Session, url: &str) -> Result<String, Error> {
			authorization_url(url)?;
			let answer = self
				.0
				.call(AUTHENTICATE, &encode_authenticate(session.id, url))?
				.await?;
			came_back(&session.return_to, utf8(&answer)?)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn sessions_round_trip_and_check_their_return_address() {
		let session = Session {
			id: 7,
			return_to: "com.example.app:/auth/native/return".into(),
		};
		assert_eq!(Session::decode(&session.encode()), Ok(session));
		let mut loopback = 9_u32.to_le_bytes().to_vec();
		loopback.extend_from_slice(b"http://127.0.0.1:50000/auth/native/return");
		assert_eq!(Session::decode(&loopback).unwrap().id, 9);

		for bad in [
			&b""[..],
			b"\x01\x00\x00",
			b"\x01\x00\x00\x00",
			b"\x01\x00\x00\x00\xff",
		] {
			assert_eq!(Session::decode(bad), Err(Error::InvalidArgument));
		}

		let mut elsewhere = 1_u32.to_le_bytes().to_vec();
		elsewhere.extend_from_slice(b"https://example.com/elsewhere");
		assert_eq!(Session::decode(&elsewhere), Err(Error::InvalidArgument));
	}

	#[test]
	fn return_addresses_are_the_apps_own_shapes() {
		for good in [
			"com.example.app:/auth/native/return",
			"http://127.0.0.1:1234/auth/native/return",
			"http://[::1]:1234/auth/native/return",
			"https://example.com/auth/native/return",
		] {
			assert!(return_address(good).is_ok(), "{good}");
		}

		for bad in [
			"com.example.app://auth/native/return",
			"com.example.app:/auth/native/return?x=1",
			"example:/auth/native/return",
			"Com.example.app:/auth/native/other",
			"com.1example.app:/auth/native/return",
			"javascript:/auth/native/return",
			"http://127.0.0.1/auth/native/return",
			"http://localhost:1234/auth/native/return",
			"http://10.0.0.1:1234/auth/native/return",
			"https://example.com/auth/native/return#x",
			"https://a:b@example.com/auth/native/return",
			"https://example.com/auth/native/return ",
		] {
			assert_eq!(return_address(bad), Err(Error::InvalidArgument), "{bad}");
		}

		assert_eq!(
			scheme_return("com.Example.App").unwrap(),
			"com.example.app:/auth/native/return"
		);
		assert_eq!(scheme_return("app"), Err(Error::InvalidArgument));
		assert_eq!(scheme_return("com.my-app"), Err(Error::InvalidArgument));
	}

	#[test]
	fn only_provider_pages_open() {
		for good in [
			"https://accounts.google.com/o/oauth2/v2/auth?state=x",
			"http://localhost:8080/authorize",
			"http://127.0.0.1:8080/authorize",
		] {
			assert!(authorization_url(good).is_ok(), "{good}");
		}

		for bad in [
			"http://example.com/authorize",
			"file:///etc/passwd",
			"javascript:alert(1)",
			"https://user:pass@example.com/",
			"ftp://example.com/authorize",
			"",
		] {
			assert_eq!(authorization_url(bad), Err(Error::InvalidArgument), "{bad}");
		}

		let payload = encode_authenticate(3, "https://example.com/authorize");
		let (id, url) = decode_authenticate(&payload).unwrap();
		assert_eq!((id, url.as_str()), (3, "https://example.com/authorize"));
		assert_eq!(
			decode_authenticate(&encode_authenticate(3, "data:text/html,x")),
			Err(Error::InvalidArgument)
		);
		assert_eq!(decode_authenticate(&[1, 2]), Err(Error::InvalidArgument));
		let long = format!("https://example.com/{}", "a".repeat(MAX_URL_BYTES));
		assert_eq!(authorization_url(&long), Err(Error::InvalidArgument));
	}

	#[test]
	fn the_browser_must_come_back_to_the_sessions_address() {
		let return_to = "http://127.0.0.1:4000/auth/native/return";
		assert_eq!(
			came_back(
				return_to,
				"http://127.0.0.1:4000/auth/native/return?code=a&state=b"
			),
			Ok("http://127.0.0.1:4000/auth/native/return?code=a&state=b".into())
		);

		for wrong in [
			"http://127.0.0.1:4001/auth/native/return?code=a",
			"http://127.0.0.2:4000/auth/native/return?code=a",
			"http://127.0.0.1:4000/auth/native/other?code=a",
			"http://127.0.0.1:4000/auth/native/return?code=a#frag",
			"https://127.0.0.1:4000/auth/native/return?code=a",
		] {
			assert_eq!(
				came_back(return_to, wrong),
				Err(Error::InvalidArgument),
				"{wrong}"
			);
		}

		let scheme = "com.example.app:/auth/native/return";
		assert!(came_back(scheme, "com.example.app:/auth/native/return?state=x").is_ok());
		assert!(came_back(scheme, "com.evil.app:/auth/native/return?state=x").is_err());
	}

	#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
	#[test]
	fn the_loopback_listener_answers_only_its_return_path() {
		use std::io::Read;
		use std::io::Write;
		use std::time::Duration;

		let loopback = desktop::Loopback::bind().unwrap();
		let port: u16 = loopback
			.return_to()
			.trim_start_matches("http://127.0.0.1:")
			.split('/')
			.next()
			.unwrap()
			.parse()
			.unwrap();
		let browser = std::thread::spawn(move || {
			let mut answers = Vec::new();

			for request in [
				"GET /favicon.ico HTTP/1.1\r\nHost: x\r\n\r\n",
				"POST /auth/native/return HTTP/1.1\r\n\r\n",
				"GET /auth/native/return?state=s&code=c HTTP/1.1\r\nHost: x\r\n\r\n",
			] {
				let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
				stream.write_all(request.as_bytes()).unwrap();
				let mut answer = String::new();
				stream.read_to_string(&mut answer).unwrap();
				answers.push(answer);
			}

			answers
		});
		let address = loopback.wait(Duration::from_secs(10)).unwrap();
		assert_eq!(
			address,
			format!("http://127.0.0.1:{port}/auth/native/return?state=s&code=c")
		);
		let answers = browser.join().unwrap();
		assert!(answers[0].starts_with("HTTP/1.1 404"));
		assert!(answers[1].starts_with("HTTP/1.1 400"));
		assert!(answers[2].starts_with("HTTP/1.1 200"));
		assert!(answers[2].contains("Cache-Control: no-store"));
		assert!(answers[2].contains("go back to the app"));

		// Nobody comes back.
		let quiet = desktop::Loopback::bind().unwrap();
		assert_eq!(quiet.wait(Duration::from_millis(60)), Err(Error::Cancelled));
	}

	#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
	#[test]
	fn the_desktop_plugin_checks_its_calls_before_opening_anything() {
		use leptoskit_bridge::Bytes;
		use leptoskit_bridge::Plugin;

		let mut plugin = desktop::WebAuth::default();
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		assert_eq!(
			plugin.invoke(BEGIN, Bytes::from_static(b"x")),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			plugin.invoke(MethodId(9), Bytes::new()),
			Err(Error::Unavailable)
		);
		// Without a session, or for another one, nothing opens.
		assert_eq!(
			plugin.invoke(
				AUTHENTICATE,
				Bytes::from(encode_authenticate(1, "https://example.com/"))
			),
			Err(Error::InvalidCall)
		);
		let session = Session::decode(&plugin.invoke(BEGIN, Bytes::new()).unwrap()).unwrap();
		assert!(session.return_to.starts_with("http://127.0.0.1:"));

		assert_eq!(
			plugin.invoke(
				AUTHENTICATE,
				Bytes::from(encode_authenticate(session.id + 1, "https://example.com/"))
			),
			Err(Error::InvalidCall)
		);
		// A page that isn't a provider's never reaches the launcher.
		let session = Session::decode(&plugin.invoke(BEGIN, Bytes::new()).unwrap()).unwrap();
		assert_eq!(
			plugin.invoke(
				AUTHENTICATE,
				Bytes::from(encode_authenticate(session.id, "file:///etc/passwd"))
			),
			Err(Error::InvalidArgument)
		);
	}
}
