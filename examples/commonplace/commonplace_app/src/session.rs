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

/// The token kept from the last sign-in.
pub async fn kept() -> Option<String> {
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
	#[cfg(not(all(target_arch = "wasm32", feature = "native", not(feature = "web"))))]
	let _ = token;
}
