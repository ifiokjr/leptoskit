//! Development guests: how `leptoskit dev` reloads native apps.
//!
//! `leptoskit dev` builds each platform's Wasm guest on every change and
//! serves it to the apps it launched, whether they run on this machine, in a
//! simulator or emulator, or on a device. A debug build launched with the
//! guest URL in [`GUEST_ENV`] (an intent extra named [`GUEST_EXTRA`] on
//! Android) follows it with [`DevGuests`] and swaps each new guest in without
//! being reinstalled.
//!
//! The protocol is one long-polled request. `GET <url>?after=<sha256>`
//! answers `200 OK` with a guest whose SHA-256, sent as its `ETag`, differs
//! from `after`, as soon as the server has one. With nothing newer, it
//! answers `204 No Content` after holding the request for [`WAIT`]. Without
//! `after`, the current guest is returned at once.
//!
//! Development guests are unsigned, so only debug builds accept them: in a
//! release build [`DevGuests::new`] fails, however the app was launched.

use std::time::Duration;

/// The environment variable carrying the guest URL to an app `leptoskit dev`
/// launched on a desktop or in the iOS Simulator.
pub const GUEST_ENV: &str = "LEPTOSKIT_DEV_GUEST";

/// The Android intent extra carrying the guest URL.
pub const GUEST_EXTRA: &str = "leptoskit.dev_guest";

/// How long the server holds a request with no newer guest to send.
pub const WAIT: Duration = Duration::from_secs(25);

/// The path of a platform's guest on the development server.
#[must_use]
pub fn guest_path(platform: &str) -> String {
	format!("/_leptoskit/dev/{platform}/guest")
}

#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
pub use client::DevGuests;
#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
pub use client::follow_environment;

#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
mod client {
	use std::sync::mpsc;
	use std::time::Duration;

	use super::GUEST_ENV;
	use super::WAIT;
	use crate::CodePushError;
	use crate::hash_bytes;
	use crate::installed::MAX_BUNDLE_BYTES;

	/// Time beyond [`WAIT`] for the server's answer to arrive.
	const RESPONSE_MARGIN: Duration = Duration::from_secs(10);

	/// How long to wait before asking an unreachable server again.
	const RETRY: Duration = Duration::from_secs(1);

	/// Follows the guest URL in [`GUEST_ENV`] on a background thread, for a
	/// desktop app `leptoskit dev` launched, and returns the receiver its new
	/// guests arrive on. Poll it from the UI thread. Returns `None` when the
	/// variable is unset, and reports why on stderr when this build cannot
	/// follow it. The thread ends once the receiver is dropped.
	#[must_use]
	pub fn follow_environment(running: &[u8]) -> Option<mpsc::Receiver<Vec<u8>>> {
		let url = std::env::var(GUEST_ENV).ok()?;
		let mut guests = DevGuests::new(&url, running)
			.inspect_err(|error| eprintln!("leptoskit dev: {error}"))
			.ok()?;
		let (sender, receiver) = mpsc::channel();
		std::thread::Builder::new()
			.name("leptoskit-dev-guests".to_owned())
			.spawn(move || {
				loop {
					match guests.next_guest() {
						Ok(Some(wasm)) => {
							if sender.send(wasm).is_err() {
								return;
							}
						}
						Ok(None) => {}
						Err(error) => {
							eprintln!("leptoskit dev: {error}");
							std::thread::sleep(RETRY);
						}
					}
				}
			})
			.inspect_err(|error| eprintln!("leptoskit dev: {error}"))
			.ok()?;
		Some(receiver)
	}

	/// Follows a development server's guest for one platform. Each
	/// [`Self::next_guest`] blocks, so call it on a background thread, never
	/// the UI thread.
	pub struct DevGuests {
		agent: ureq::Agent,
		url: String,
		running: String,
	}

	impl DevGuests {
		/// Follows the guest at `url`, starting from the guest the app already
		/// runs, so the first [`Self::next_guest`] waits for a change instead
		/// of returning the same module.
		///
		/// # Errors
		///
		/// Fails in release builds, which never run unsigned code, and for a
		/// URL that is not plain `http` or `https` without a query.
		pub fn new(url: &str, running: &[u8]) -> Result<Self, CodePushError> {
			if !cfg!(debug_assertions) {
				return Err(CodePushError::Activation(
					"development guests are only accepted by debug builds".to_owned(),
				));
			}

			let scheme = url.starts_with("http://") || url.starts_with("https://");

			if !scheme || url.contains(['?', '#']) {
				return Err(CodePushError::Transport(format!(
					"invalid development guest URL {url:?}"
				)));
			}

			let config = ureq::Agent::config_builder()
				.max_redirects(0)
				.http_status_as_error(false)
				.timeout_connect(Some(Duration::from_secs(5)))
				.timeout_recv_response(Some(WAIT + RESPONSE_MARGIN))
				.build();

			Ok(Self {
				agent: config.into(),
				url: url.to_owned(),
				running: hash_bytes(running),
			})
		}

		/// The SHA-256 of the guest the app runs, as lowercase hex.
		#[must_use]
		pub fn running(&self) -> &str {
			&self.running
		}

		/// Waits up to [`WAIT`] for a guest other than the running one. The
		/// returned guest counts as running from then on, so one that fails to
		/// start is not offered again; the next change is.
		///
		/// # Errors
		///
		/// Fails when the server is unreachable, answers with an unexpected
		/// status, or sends bytes that do not match their `ETag`.
		pub fn next_guest(&mut self) -> Result<Option<Vec<u8>>, CodePushError> {
			let url = format!("{}?after={}", self.url, self.running);
			let mut response = self
				.agent
				.get(&url)
				.call()
				.map_err(|error| CodePushError::Transport(error.to_string()))?;

			match response.status().as_u16() {
				204 => return Ok(None),
				200 => {}
				status => {
					return Err(CodePushError::Transport(format!(
						"HTTP {status} from {}",
						self.url
					)));
				}
			}

			let expected = response
				.headers()
				.get(ureq::http::header::ETAG)
				.and_then(|value| value.to_str().ok())
				.map(|value| value.trim_matches('"').to_owned())
				.ok_or_else(|| {
					CodePushError::Transport("the development guest has no ETag".to_owned())
				})?;
			let wasm = response
				.body_mut()
				.with_config()
				.limit(MAX_BUNDLE_BYTES)
				.read_to_vec()
				.map_err(|error| CodePushError::Transport(error.to_string()))?;
			let actual = hash_bytes(&wasm);

			if actual != expected {
				return Err(CodePushError::HashMismatch {
					path: self.url.clone(),
					expected,
					actual,
				});
			}

			self.running = actual;
			Ok(Some(wasm))
		}
	}
}
