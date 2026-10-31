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

/// What a native app's device does for sign-in: passkeys and Sign in with
/// Apple through the passkeys plugin, and other providers in the system
/// browser through the web-auth plugin. Passkey autofill isn't offered: the
/// native text field has no passkey suggestions.
#[cfg(all(feature = "guest", target_arch = "wasm32"))]
#[derive(Default)]
pub(crate) struct System {
	/// Stops the passkey request that's waiting.
	waiting: Arc<Mutex<Option<futures::future::AbortHandle>>>,
}

#[cfg(all(feature = "guest", target_arch = "wasm32"))]
mod system {
	use leptoskit_bridge::Error;
	use leptoskit_plugin_passkeys::guest::Passkeys;
	use leptoskit_plugin_web_auth::guest::WebAuth;

	use super::*;
	use crate::CreationChallengeResponse;
	use crate::PublicKeyCredential;
	use crate::RegisterPublicKeyCredential;
	use crate::RequestChallengeResponse;
	use crate::client::protocol::AppleId;
	use crate::client::protocol::BrowserSession;
	use crate::client::protocol::Platform;

	/// A plugin the build doesn't have.
	fn missing(plugin: &str) -> ErrorResponse {
		error(
			"config",
			&format!("this needs the {plugin} plugin: grant \"{plugin}\" in `[native] plugins`"),
		)
	}

	/// What a plugin's failure means to the person.
	fn failed(failure: Error, otherwise: &str) -> ErrorResponse {
		match failure {
			Error::Cancelled => error("cancelled", "it was closed before it finished"),
			Error::Busy => error("cancelled", "another sign-in is already open"),
			Error::Unavailable => {
				error(
					"unsupported",
					"this device can't do that; sign in another way",
				)
			}
			Error::PermissionDenied => {
				error(
					"config",
					"the plugin isn't granted that; check `[native] plugins`",
				)
			}
			_ => error(otherwise, "the device couldn't finish it; try again"),
		}
	}

	fn options<T: serde::Serialize>(public_key: &T) -> Result<String, ErrorResponse> {
		serde_json::to_string(public_key)
			.map_err(|_| error("passkey_failed", "the passkey options didn't make sense"))
	}

	fn answer<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, ErrorResponse> {
		serde_json::from_str(json).map_err(|_| {
			error(
				"passkey_failed",
				"the device's passkey answer didn't make sense",
			)
		})
	}

	impl System {
		fn abortable<T: Send + 'static>(
			&self,
			work: impl Future<Output = Result<T, ErrorResponse>> + Send + 'static,
		) -> BoxFuture<'static, Result<T, ErrorResponse>> {
			let (work, handle) = futures::future::abortable(work);
			*self.waiting.lock().unwrap_or_else(PoisonError::into_inner) = Some(handle);
			Box::pin(async move {
				work.await
					.unwrap_or_else(|_| Err(error("cancelled", "it was stopped")))
			})
		}
	}

	impl Platform for System {
		fn create(
			&self,
			options: CreationChallengeResponse,
		) -> BoxFuture<'static, Result<RegisterPublicKeyCredential, ErrorResponse>> {
			self.abortable(async move {
				let json = self::options(&options.public_key)?;
				let plugin = Passkeys::new().map_err(|_| missing("passkeys"))?;
				let made = plugin
					.create(&json)
					.await
					.map_err(|failure| failed(failure, "passkey_failed"))?;
				answer(&made)
			})
		}

		fn get(
			&self,
			options: RequestChallengeResponse,
			autofill: bool,
		) -> BoxFuture<'static, Result<PublicKeyCredential, ErrorResponse>> {
			if autofill {
				return Box::pin(std::future::ready(crate::client::protocol::unsupported()));
			}

			self.abortable(async move {
				let json = self::options(&options.public_key)?;
				let plugin = Passkeys::new().map_err(|_| missing("passkeys"))?;
				let signed = plugin
					.get(&json)
					.await
					.map_err(|failure| failed(failure, "passkey_failed"))?;
				answer(&signed)
			})
		}

		fn cancel(&self) {
			if let Some(waiting) = self
				.waiting
				.lock()
				.unwrap_or_else(PoisonError::into_inner)
				.take()
			{
				waiting.abort();
			}
		}

		fn open(&self, _url: &str) -> Result<(), ErrorResponse> {
			crate::client::protocol::unsupported()
		}

		fn query(&self) -> Option<String> {
			None
		}

		/// Without the plugin, as on Windows, Linux, and HarmonyOS, passkey
		/// calls answer `unsupported` before asking the server anything.
		fn passkeys(&self) -> bool {
			Passkeys::new().is_ok()
		}

		fn autofill(&self) -> bool {
			false
		}

		fn begin_browser(&self) -> BoxFuture<'static, Result<BrowserSession, ErrorResponse>> {
			Box::pin(async {
				let plugin = WebAuth::new().map_err(|_| missing("web-auth"))?;
				let session = plugin
					.begin()
					.await
					.map_err(|failure| failed(failure, "provider"))?;
				Ok(BrowserSession {
					id: session.id,
					return_to: session.return_to,
				})
			})
		}

		fn browse(
			&self,
			session: BrowserSession,
			url: String,
		) -> BoxFuture<'static, Result<String, ErrorResponse>> {
			Box::pin(async move {
				let plugin = WebAuth::new().map_err(|_| missing("web-auth"))?;
				let session = leptoskit_plugin_web_auth::Session {
					id: session.id,
					return_to: session.return_to,
				};
				plugin
					.authenticate(&session, &url)
					.await
					.map_err(|failure| failed(failure, "provider"))
			})
		}

		fn has_apple_id(&self) -> bool {
			Passkeys::new().is_ok()
		}

		fn apple_id(&self, nonce: String) -> BoxFuture<'static, Result<AppleId, ErrorResponse>> {
			Box::pin(async move {
				let plugin = Passkeys::new().map_err(|_| missing("passkeys"))?;
				let credential = plugin
					.apple_id(&nonce)
					.await
					.map_err(|failure| failed(failure, "provider"))?;
				Ok(AppleId {
					user: credential.user(),
					id_token: credential.id_token,
				})
			})
		}
	}
}
