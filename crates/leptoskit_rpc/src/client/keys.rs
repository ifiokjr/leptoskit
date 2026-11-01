//! Idempotency keys: 128 random bits, as 32 hex digits, from the best
//! source each platform has.

use std::fmt::Write;

/// A fresh idempotency key.
pub(crate) async fn new_key() -> String {
	let mut bytes = [0_u8; 16];
	fill(&mut bytes).await;
	bytes
		.iter()
		.fold(String::with_capacity(32), |mut key, byte| {
			let _ = write!(key, "{byte:02x}");
			key
		})
}

/// On native targets, from the operating system, through the keys the
/// standard library seeds its hash maps with.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::unused_async, reason = "a guest's source is async")]
async fn fill(bytes: &mut [u8; 16]) {
	use std::hash::BuildHasher;

	let state = std::collections::hash_map::RandomState::new();
	let high = state.hash_one(0_u8).to_le_bytes();
	let low = std::collections::hash_map::RandomState::new()
		.hash_one(1_u8)
		.to_le_bytes();
	bytes[..8].copy_from_slice(&high);
	bytes[8..].copy_from_slice(&low);
}

/// In the browser, from `crypto.getRandomValues`.
#[cfg(all(target_arch = "wasm32", feature = "web", not(feature = "guest")))]
#[allow(clippy::unused_async, reason = "a guest's source is async")]
async fn fill(bytes: &mut [u8; 16]) {
	let filled = web_sys::window()
		.and_then(|window| window.crypto().ok())
		.and_then(|crypto| crypto.get_random_values_with_u8_array(bytes).ok());

	if filled.is_none() {
		fallback(bytes);
	}
}

/// In a native app, from the platform, through the crypto plugin when the
/// app has it.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
async fn fill(bytes: &mut [u8; 16]) {
	let random = match leptoskit_plugin_crypto::guest::Crypto::new() {
		Ok(crypto) => crypto.random_bytes(16).await.ok(),
		Err(_) => None,
	};

	match random {
		Some(random) if random.len() == 16 => bytes.copy_from_slice(&random),
		_ => fallback(bytes),
	}
}

/// Anywhere else.
#[cfg(all(target_arch = "wasm32", not(any(feature = "web", feature = "guest"))))]
#[allow(clippy::unused_async, reason = "a guest's source is async")]
async fn fill(bytes: &mut [u8; 16]) {
	fallback(bytes);
}

/// The time, a counter, and an address: unique on this device, though
/// guessable, for a guest without the crypto plugin. The server keeps each
/// signed-in caller's keys apart, so only anonymous calls rely on them being
/// hard to guess.
#[cfg(target_arch = "wasm32")]
fn fallback(bytes: &mut [u8; 16]) {
	use std::sync::atomic::AtomicU64;
	use std::sync::atomic::Ordering;

	static COUNTER: AtomicU64 = AtomicU64::new(0);
	let local = 0_u8;
	let address = std::ptr::from_ref(&local) as u64;
	let count = COUNTER.fetch_add(1, Ordering::Relaxed);
	let time = leptoskit_query::now_ms();
	bytes[..8].copy_from_slice(&(time ^ address.rotate_left(32)).to_le_bytes());
	bytes[8..].copy_from_slice(&count.wrapping_mul(0x9e37_79b9_7f4a_7c15).to_le_bytes());
}
