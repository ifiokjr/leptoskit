//! The development guest server: the latest Wasm guest of every native
//! platform `leptoskit dev` runs, long-polled by the apps it launched. The
//! protocol is [`leptoskit_codepush::dev`]'s.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::Path;
use axum::extract::Query;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use leptoskit_codepush::dev::WAIT;
use serde::Deserialize;
use tokio::sync::watch;

/// One built guest and its SHA-256.
#[derive(Debug, Clone)]
struct Guest {
	wasm: Bytes,
	hash: String,
}

/// The latest guest per platform. Cloning shares the same channels.
#[derive(Debug, Clone)]
pub struct Guests {
	channels: Arc<BTreeMap<String, watch::Sender<Option<Guest>>>>,
	/// How long a request with nothing newer is held: [`WAIT`] outside tests.
	wait: Duration,
}

impl Guests {
	/// Channels for exactly these platforms; requests for any other platform
	/// answer `404`.
	pub fn new(platforms: impl IntoIterator<Item = String>) -> Self {
		let channels = platforms
			.into_iter()
			.map(|platform| (platform, watch::channel(None).0))
			.collect();
		Self {
			channels: Arc::new(channels),
			wait: WAIT,
		}
	}

	/// Offers `wasm` to every app following `platform`. Publishing the guest
	/// a platform already has wakes nobody.
	pub fn publish(&self, platform: &str, wasm: Vec<u8>) {
		let Some(channel) = self.channels.get(platform) else {
			return;
		};
		let hash = leptoskit_codepush::hash_bytes(&wasm);
		channel.send_if_modified(|current| {
			if current.as_ref().is_some_and(|guest| guest.hash == hash) {
				return false;
			}

			*current = Some(Guest {
				wasm: Bytes::from(wasm),
				hash,
			});
			true
		});
	}

	/// The routes, at [`leptoskit_codepush::dev::guest_path`] for each
	/// platform.
	pub fn router(&self) -> Router {
		Router::new()
			.route("/_leptoskit/dev/{platform}/guest", get(guest))
			.with_state(self.clone())
	}
}

#[derive(Debug, Deserialize)]
struct GuestQuery {
	after: Option<String>,
}

/// Answers with the platform's guest once its hash differs from `after`, or
/// `204 No Content` when none does within the wait.
async fn guest(
	State(guests): State<Guests>,
	Path(platform): Path<String>,
	Query(query): Query<GuestQuery>,
) -> Response {
	let Some(channel) = guests.channels.get(&platform) else {
		return StatusCode::NOT_FOUND.into_response();
	};
	let mut receiver = channel.subscribe();
	let deadline = tokio::time::Instant::now() + guests.wait;

	loop {
		let offered = receiver
			.borrow_and_update()
			.clone()
			.filter(|guest| query.after.as_deref() != Some(guest.hash.as_str()));

		if let Some(guest) = offered {
			return (
				[
					(header::CONTENT_TYPE, "application/wasm".to_owned()),
					(header::ETAG, format!("\"{}\"", guest.hash)),
					(header::CACHE_CONTROL, "no-store".to_owned()),
				],
				guest.wasm,
			)
				.into_response();
		}

		match tokio::time::timeout_at(deadline, receiver.changed()).await {
			Ok(changed) => changed.expect("the handler's own guests keep the channel open"),
			Err(_) => return StatusCode::NO_CONTENT.into_response(),
		}
	}
}

#[cfg(test)]
mod tests {
	use std::net::SocketAddr;

	use leptoskit_codepush::dev::DevGuests;
	use leptoskit_codepush::dev::guest_path;

	use super::*;

	/// Serves `guests`, holding idle requests briefly, on an ephemeral
	/// loopback port.
	fn serve(mut guests: Guests) -> (tokio::runtime::Runtime, SocketAddr) {
		guests.wait = Duration::from_millis(200);
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let listener = runtime
			.block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
			.unwrap();
		let address = listener.local_addr().unwrap();
		let router = guests.router();
		runtime.spawn(async move { axum::serve(listener, router).await.unwrap() });
		(runtime, address)
	}

	#[test]
	fn an_app_receives_each_new_guest_and_skips_the_one_it_runs() {
		let guests = Guests::new(["ios", "android"].map(String::from));
		guests.publish("ios", b"first".to_vec());
		let (_runtime, address) = serve(guests.clone());
		let url = format!("http://{address}{}", guest_path("ios"));

		let mut client = DevGuests::new(&url, b"embedded").unwrap();
		assert_eq!(client.next_guest().unwrap().as_deref(), Some(&b"first"[..]));
		assert_eq!(
			client.next_guest().unwrap(),
			None,
			"nothing newer than the running guest"
		);

		let waiting = std::thread::spawn(move || (client.next_guest(), client));
		std::thread::sleep(Duration::from_millis(50));
		guests.publish("ios", b"first".to_vec());
		guests.publish("android", b"other platform".to_vec());
		guests.publish("ios", b"second".to_vec());
		let (second, client) = waiting.join().unwrap();
		assert_eq!(
			second.unwrap().as_deref(),
			Some(&b"second"[..]),
			"a waiting request wakes for its own platform's new guest"
		);
		assert_eq!(client.running(), leptoskit_codepush::hash_bytes(b"second"));
	}

	#[test]
	fn an_app_starting_from_the_published_guest_waits_and_unknown_platforms_are_missing() {
		let guests = Guests::new([String::from("macos")]);
		guests.publish("macos", b"current".to_vec());
		guests.publish("windows", b"not served".to_vec());
		let (_runtime, address) = serve(guests);

		let url = format!("http://{address}{}", guest_path("macos"));
		let mut client = DevGuests::new(&url, b"current").unwrap();
		assert_eq!(client.next_guest().unwrap(), None);

		let url = format!("http://{address}{}", guest_path("windows"));
		let mut client = DevGuests::new(&url, b"anything").unwrap();
		let error = client.next_guest().unwrap_err();
		assert!(error.to_string().contains("HTTP 404"), "{error}");
	}
}
