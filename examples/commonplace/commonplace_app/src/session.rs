//! Keeping the session token between launches.
//!
//! The website never sees its token: the browser holds it in a cookie
//! scripts can't read. The native apps keep theirs in the platform's secure
//! store (the Keychain, or the Android Keystore).

/// The service and key the token is stored under.
#[cfg(all(target_arch = "wasm32", feature = "native", not(feature = "web")))]
const SERVICE: &str = "commonplace";
#[cfg(all(target_arch = "wasm32", feature = "native", not(feature = "web")))]
const KEY: &str = "session";

/// Where the extension keeps the token: extension storage, which its pages
/// and its worker share, and sites can't read.
#[cfg(all(target_arch = "wasm32", feature = "extension"))]
const EXTENSION_KEY: &str = "commonplace.session";

/// The token kept from the last sign-in.
pub async fn kept() -> Option<String> {
	#[cfg(all(target_arch = "wasm32", feature = "extension"))]
	{
		return leptoskit::extension::storage::LOCAL
			.get::<String>(EXTENSION_KEY)
			.await
			.ok()
			.flatten();
	}
	#[cfg(all(target_arch = "wasm32", feature = "native", not(feature = "web")))]
	{
		use leptoskit_plugin_secure_store::Key;
		use leptoskit_plugin_secure_store::guest::SecureStore;

		let key = Key::new(SERVICE, KEY).ok()?;
		let secret = SecureStore::new().ok()?.get(key).await.ok()??;
		return String::from_utf8(secret.to_vec()).ok();
	}
	#[allow(unreachable_code)]
	None
}

/// Keeps `token` for the next launch, or forgets the kept one.
pub async fn keep(token: Option<String>) {
	#[cfg(all(target_arch = "wasm32", feature = "extension"))]
	{
		let _ = match &token {
			Some(token) => {
				leptoskit::extension::storage::LOCAL
					.set(EXTENSION_KEY, token)
					.await
			}
			None => {
				leptoskit::extension::storage::LOCAL
					.remove(EXTENSION_KEY)
					.await
			}
		};
		// The other surfaces and the worker hold their own copy.
		let _ = leptoskit::extension::runtime::send::<_, commonplace_shared::ExtensionReply>(
			&commonplace_shared::ExtensionMessage::SessionChanged,
		)
		.await;
	}
	#[cfg(all(target_arch = "wasm32", feature = "native", not(feature = "web")))]
	{
		use leptoskit_plugin_secure_store::Key;
		use leptoskit_plugin_secure_store::guest::SecureStore;

		let (Ok(key), Ok(store)) = (Key::new(SERVICE, KEY), SecureStore::new()) else {
			return;
		};
		let _ = match token {
			Some(token) => store.set(key, token.as_bytes()).await,
			None => store.delete(key).await,
		};
	}
	#[cfg(not(any(
		all(target_arch = "wasm32", feature = "native", not(feature = "web")),
		all(target_arch = "wasm32", feature = "extension")
	)))]
	let _ = token;
}
