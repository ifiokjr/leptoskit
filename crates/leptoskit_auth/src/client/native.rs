//! The client in a native app, which has no cookie jar. It holds the access
//! token in memory only, and the refresh token in the platform's secure
//! store, so the session outlives a restart.
//!
//! Before the access token runs out, or after the server refuses it, the
//! client trades the refresh token for a new pair. Only one trade runs at a
//! time: the server takes a refresh token's second use, after a short grace,
//! as a sign that someone copied it, and ends the session.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use leptoskit_query::BoxFuture;

use super::protocol::Credentials;
use super::protocol::Endpoint;
use super::protocol::Http;
use super::protocol::error;
use crate::ErrorResponse;
use crate::Tokens;
use crate::Transport;

/// How long before the access token runs out the client trades it, at
/// most: a request sent just before then still arrives in time.
const RENEW_EARLY_MS: u64 = 60_000;

/// Where the refresh token is kept between runs.
pub(crate) trait SecretStore: Send + Sync + 'static {
	fn get(&self) -> BoxFuture<'_, Result<Option<String>, ErrorResponse>>;
	fn set(&self, refresh_token: String) -> BoxFuture<'_, Result<(), ErrorResponse>>;
	fn delete(&self) -> BoxFuture<'_, Result<(), ErrorResponse>>;
}

/// The access token, and when to trade it, in milliseconds since the epoch.
struct Access {
	token: String,
	renew_at: u64,
}

/// A native app's credentials.
pub(crate) struct TokenKeeper<S> {
	store: S,
	clock: Arc<dyn Fn() -> u64 + Send + Sync>,
	access: Mutex<Option<Access>>,
	/// The second step of signing in's token, in memory only: it lasts five
	/// minutes, and signing in starts again after a restart.
	second_factor: Mutex<Option<String>>,
	/// Held while a trade is in flight, so concurrent requests wait for it
	/// instead of starting their own.
	renewing: futures::lock::Mutex<()>,
}

impl<S: SecretStore> TokenKeeper<S> {
	/// Keeps the refresh token in `store`, and reads the time, in
	/// milliseconds since the epoch, from `clock`.
	pub(crate) fn new(store: S, clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
		Self {
			store,
			clock: Arc::new(clock),
			access: Mutex::new(None),
			second_factor: Mutex::new(None),
			renewing: futures::lock::Mutex::new(()),
		}
	}

	#[cfg(test)]
	pub(crate) fn store(&self) -> &S {
		&self.store
	}

	fn access(&self) -> std::sync::MutexGuard<'_, Option<Access>> {
		self.access.lock().unwrap_or_else(PoisonError::into_inner)
	}

	/// The access token, as a header, if it has a while left.
	fn fresh(&self) -> Option<String> {
		let now = (self.clock)();
		self.access()
			.as_ref()
			.filter(|access| now < access.renew_at)
			.map(|access| bearer(&access.token))
	}

	/// The access token, as a header, however long it has left.
	fn current(&self) -> Option<String> {
		self.access().as_ref().map(|access| bearer(&access.token))
	}

	async fn hold(&self, tokens: Tokens) -> Result<(), ErrorResponse> {
		let lifetime = tokens.expires_in.saturating_mul(1000);
		let early = RENEW_EARLY_MS.min(lifetime / 2);
		*self.access() = Some(Access {
			token: tokens.access_token,
			renew_at: (self.clock)().saturating_add(lifetime - early),
		});
		self.store.set(tokens.refresh_token).await
	}

	async fn clear(&self) {
		*self.access() = None;
		// Nothing more can be done if the store won't forget, and the server
		// has ended the session either way.
		let _ = self.store.delete().await;
	}

	/// Trades the stored refresh token for a new pair; call it holding
	/// `renewing`. `false` when there's no session left to renew.
	async fn renew<H: Http>(&self, server: &Endpoint<H>) -> Result<bool, ErrorResponse> {
		let Some(refresh_token) = self.store.get().await? else {
			*self.access() = None;
			return Ok(false);
		};

		match server.refresh(refresh_token).await {
			Ok(tokens) => self.hold(tokens).await.map(|()| true),
			Err(refused) if refused.error == "signed_out" => {
				self.clear().await;
				Ok(false)
			}
			// The token may still be good: a lost answer can be retried with
			// it for a short while.
			Err(error) => Err(error),
		}
	}
}

impl<S: SecretStore> Credentials for TokenKeeper<S> {
	const TRANSPORT: Transport = Transport::Bearer;

	fn authorization<'a, H: Http>(
		&'a self,
		server: &'a Endpoint<H>,
	) -> BoxFuture<'a, Result<Option<String>, ErrorResponse>> {
		Box::pin(async move {
			if let Some(header) = self.fresh() {
				return Ok(Some(header));
			}

			let _renewing = self.renewing.lock().await;

			// Another request may have renewed while this one waited.
			if let Some(header) = self.fresh() {
				return Ok(Some(header));
			}

			Ok(if self.renew(server).await? {
				self.current()
			} else {
				None
			})
		})
	}

	fn refused<'a, H: Http>(
		&'a self,
		server: &'a Endpoint<H>,
		sent: Option<String>,
	) -> BoxFuture<'a, Result<bool, ErrorResponse>> {
		Box::pin(async move {
			let _renewing = self.renewing.lock().await;
			let current = self.current();

			if current.is_some() && current != sent {
				// Renewed since this request was sent.
				return Ok(true);
			}

			self.renew(server).await
		})
	}

	fn keep(&self, tokens: Option<Tokens>) -> BoxFuture<'_, Result<(), ErrorResponse>> {
		Box::pin(async move {
			let tokens = tokens.ok_or_else(|| {
				error(
					"server",
					"the server signed in without sending tokens; is it up to date?",
				)
			})?;
			self.hold(tokens).await
		})
	}

	fn forget(&self) -> BoxFuture<'_, ()> {
		self.hold_second_factor(None);
		Box::pin(self.clear())
	}

	fn hold_second_factor(&self, token: Option<String>) {
		*self
			.second_factor
			.lock()
			.unwrap_or_else(PoisonError::into_inner) = token;
	}

	fn second_factor(&self) -> Option<String> {
		self.second_factor
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.clone()
	}
}

fn bearer(token: &str) -> String {
	format!("Bearer {token}")
}

/// Sends requests through the host's HTTP plugin.
#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub(crate) struct Plugin;

#[cfg(all(feature = "guest", target_arch = "wasm32"))]
impl Http for Plugin {
	fn send(
		&self,
		request: super::protocol::HttpRequest,
	) -> BoxFuture<'static, Result<super::protocol::HttpResponse, ErrorResponse>> {
		use leptoskit_plugin_http::Header;
		use leptoskit_plugin_http::Method;
		use leptoskit_plugin_http::guest::HttpClient;

		Box::pin(async move {
			if request.url.starts_with('/') {
				return Err(error(
					"config",
					"a native app needs the server's origin: call \
					 `leptoskit::auth::client::configure` with it when the app starts",
				));
			}

			let headers = request
				.headers()
				.into_iter()
				.map(|(name, value)| {
					Header {
						name: name.to_owned(),
						value: value.as_bytes().to_vec(),
					}
				})
				.collect();
			let plugin = HttpClient::connect().map_err(http_failed)?;
			let response = plugin
				.execute(leptoskit_plugin_http::Request {
					method: match request.method {
						super::protocol::Method::Get => Method::Get,
						super::protocol::Method::Post => Method::Post,
					},
					url: request.url,
					headers,
					body: request.body.unwrap_or_default().into_bytes(),
				})
				.await
				.map_err(http_failed)?;

			Ok(super::protocol::HttpResponse {
				status: response.status,
				body: response.body,
			})
		})
	}
}

#[cfg(all(feature = "guest", target_arch = "wasm32"))]
fn http_failed(failure: leptoskit_bridge::Error) -> ErrorResponse {
	use leptoskit_bridge::Error;

	match failure {
		Error::Unavailable => {
			error(
				"config",
				"sign-in needs the HTTP plugin, which this build doesn't have: grant \"http\" in \
				 `[native] plugins` (the Linux, Windows, and terminal runners can't host plugins \
				 yet)",
			)
		}
		Error::PermissionDenied => {
			error(
				"config",
				"the HTTP plugin doesn't allow the server's origin: add it to \
				 `[native.plugin-options.http] origins`",
			)
		}
		_ => error("network", "couldn't reach the server; try again"),
	}
}

/// Keeps the refresh token in the platform's secure store: the Keychain on
/// Apple platforms, and the platform's credential store elsewhere.
#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub(crate) struct Keychain;

#[cfg(all(feature = "guest", target_arch = "wasm32"))]
mod keychain {
	use leptoskit_plugin_secure_store::Key;
	use leptoskit_plugin_secure_store::guest::SecureStore;

	use super::*;

	fn store() -> Result<(SecureStore, Key<'static>), ErrorResponse> {
		let store = SecureStore::new().map_err(failed)?;
		let key = Key::new("leptoskit.auth", "refresh_token").map_err(failed)?;
		Ok((store, key))
	}

	fn failed(failure: leptoskit_bridge::Error) -> ErrorResponse {
		match failure {
			leptoskit_bridge::Error::Unavailable => {
				error(
					"config",
					"sign-in needs the secure store, which this build doesn't have: grant \
					 \"secure-store\" in `[native] plugins` (the Linux, Windows, and terminal \
					 runners can't host plugins yet)",
				)
			}
			_ => error("storage", "couldn't use the device's secure store"),
		}
	}

	impl SecretStore for Keychain {
		fn get(&self) -> BoxFuture<'_, Result<Option<String>, ErrorResponse>> {
			Box::pin(async {
				let (store, key) = store()?;
				let secret = store.get(key).await.map_err(failed)?;
				Ok(secret.and_then(|secret| String::from_utf8(secret.to_vec()).ok()))
			})
		}

		fn set(&self, refresh_token: String) -> BoxFuture<'_, Result<(), ErrorResponse>> {
			Box::pin(async move {
				let (store, key) = store()?;
				store
					.set(key, refresh_token.as_bytes())
					.await
					.map_err(failed)
			})
		}

		fn delete(&self) -> BoxFuture<'_, Result<(), ErrorResponse>> {
			Box::pin(async {
				let (store, key) = store()?;
				store.delete(key).await.map_err(failed)
			})
		}
	}
}
