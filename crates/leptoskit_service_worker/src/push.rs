//! Web Push subscriptions on a service worker registration.

use serde::Deserialize;
use serde::Serialize;

use crate::Error;
use crate::Result;
use crate::js;

/// Options for [`PushManager::subscribe`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushOptions {
	/// The VAPID application server key, as URL-safe base64 (no padding).
	pub application_server_key: String,
	/// Whether the subscription covers only this user-visible session.
	/// Defaults to `true` (user-visible pushes).
	#[serde(default = "default_user_visible")]
	pub user_visible_only: bool,
}

fn default_user_visible() -> bool {
	true
}

impl PushOptions {
	/// A user-visible subscription with the given VAPID public key.
	#[must_use]
	pub fn new(application_server_key: impl Into<String>) -> Self {
		Self {
			application_server_key: application_server_key.into(),
			user_visible_only: true,
		}
	}
}

/// A push subscription's endpoint and keys, suitable for sending to your
/// server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushSubscription {
	/// The push service endpoint URL.
	pub endpoint: String,
	/// The subscription's expiration time in milliseconds since the epoch, if
	/// any.
	#[serde(default)]
	pub expiration_time: Option<f64>,
	/// The `p256dh` and `auth` keys, as URL-safe base64.
	pub keys: PushKeys,
}

/// Client keys the push service needs to encrypt payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushKeys {
	/// The P-256 ECDH public key.
	pub p256dh: String,
	/// The authentication secret.
	pub auth: String,
}

/// The push manager on a [`Registration`](crate::Registration).
#[derive(Debug, Clone)]
pub struct PushManager {
	#[cfg(target_arch = "wasm32")]
	pub(crate) inner: wasm_bindgen::JsValue,
	#[cfg(not(target_arch = "wasm32"))]
	pub(crate) _private: (),
}

impl PushManager {
	/// Creates or returns the existing push subscription.
	///
	/// Browsers only show the permission prompt after a user gesture. The page
	/// needs a secure context, and your worker script must handle `push`
	/// events.
	///
	/// # Errors
	///
	/// Outside a browser, without permission, or when Push is unavailable.
	pub async fn subscribe(&self, options: &PushOptions) -> Result<PushSubscription> {
		#[cfg(target_arch = "wasm32")]
		{
			let key = decode_base64_url(&options.application_server_key)?;
			let opts = js_sys::Object::new();
			js_sys::Reflect::set(
				&opts,
				&"userVisibleOnly".into(),
				&wasm_bindgen::JsValue::from_bool(options.user_visible_only),
			)
			.map_err(|error| Error::Browser(js::message(&error)))?;
			let buffer = js_sys::Uint8Array::from(key.as_slice());
			js_sys::Reflect::set(&opts, &"applicationServerKey".into(), &buffer)
				.map_err(|error| Error::Browser(js::message(&error)))?;
			let value = js::call_method(&self.inner, "subscribe", &[opts.into()]).await?;
			subscription_from_js(&value)
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			let _ = options;
			js::unavailable_async("PushManager.subscribe").await
		}
	}

	/// The existing subscription, if any.
	///
	/// # Errors
	///
	/// Outside a browser.
	pub async fn get_subscription(&self) -> Result<Option<PushSubscription>> {
		#[cfg(target_arch = "wasm32")]
		{
			let value = js::call_method(&self.inner, "getSubscription", &[]).await?;
			if value.is_null() || value.is_undefined() {
				return Ok(None);
			}
			subscription_from_js(&value).map(Some)
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			js::unavailable_async("PushManager.getSubscription").await
		}
	}

	/// Unsubscribes the current push subscription, if any. Returns whether
	/// one was removed.
	///
	/// # Errors
	///
	/// Outside a browser.
	pub async fn unsubscribe(&self) -> Result<bool> {
		#[cfg(target_arch = "wasm32")]
		{
			let value = js::call_method(&self.inner, "getSubscription", &[]).await?;
			if value.is_null() || value.is_undefined() {
				return Ok(false);
			}
			let result = js::call_method(&value, "unsubscribe", &[]).await?;
			Ok(result.as_bool().unwrap_or(false))
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			js::unavailable_async("PushSubscription.unsubscribe").await
		}
	}
}

#[cfg(target_arch = "wasm32")]
fn subscription_from_js(value: &wasm_bindgen::JsValue) -> Result<PushSubscription> {
	use wasm_bindgen::JsCast;

	let endpoint = js::property(value, "endpoint")?
		.as_string()
		.ok_or_else(|| Error::Browser("push subscription has no endpoint".into()))?;
	let expiration_time = js::property(value, "expirationTime")?
		.as_f64()
		.filter(|value| !value.is_nan());
	let get_key = js_sys::Reflect::get(value, &"getKey".into())
		.ok()
		.and_then(|value| value.dyn_into::<js_sys::Function>().ok())
		.ok_or_else(|| Error::Unavailable("PushSubscription.getKey".into()))?;
	let p256dh = get_key
		.call1(value, &"p256dh".into())
		.map_err(|error| Error::Browser(js::message(&error)))?;
	let auth = get_key
		.call1(value, &"auth".into())
		.map_err(|error| Error::Browser(js::message(&error)))?;
	Ok(PushSubscription {
		endpoint,
		expiration_time,
		keys: PushKeys {
			p256dh: encode_base64_url(&buffer_to_bytes(&p256dh)?),
			auth: encode_base64_url(&buffer_to_bytes(&auth)?),
		},
	})
}

#[cfg(target_arch = "wasm32")]
fn buffer_to_bytes(value: &wasm_bindgen::JsValue) -> Result<Vec<u8>> {
	if value.is_null() || value.is_undefined() {
		return Err(Error::Browser("push key missing".into()));
	}
	let array = js_sys::Uint8Array::new(value);
	let mut bytes = vec![0_u8; array.length() as usize];
	array.copy_to(&mut bytes);
	Ok(bytes)
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn decode_base64_url(value: &str) -> Result<Vec<u8>> {
	let mut padded = value.replace('-', "+").replace('_', "/");
	while !padded.len().is_multiple_of(4) {
		padded.push('=');
	}
	base64_decode(&padded)
		.map_err(|error| Error::Json(format!("invalid applicationServerKey: {error}")))
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn encode_base64_url(bytes: &[u8]) -> String {
	base64_encode(bytes)
		.trim_end_matches('=')
		.replace('+', "-")
		.replace('/', "_")
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn base64_encode(bytes: &[u8]) -> String {
	const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
	let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
	for chunk in bytes.chunks(3) {
		let mut n = u32::from(chunk[0]) << 16;
		if chunk.len() > 1 {
			n |= u32::from(chunk[1]) << 8;
		}
		if chunk.len() > 2 {
			n |= u32::from(chunk[2]);
		}
		out.push(TABLE[((n >> 18) & 63) as usize] as char);
		out.push(TABLE[((n >> 12) & 63) as usize] as char);
		if chunk.len() > 1 {
			out.push(TABLE[((n >> 6) & 63) as usize] as char);
		} else {
			out.push('=');
		}
		if chunk.len() > 2 {
			out.push(TABLE[(n & 63) as usize] as char);
		} else {
			out.push('=');
		}
	}
	out
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn base64_decode(input: &str) -> Result<Vec<u8>, &'static str> {
	fn digit(byte: u8) -> Result<u8, &'static str> {
		match byte {
			b'A'..=b'Z' => Ok(byte - b'A'),
			b'a'..=b'z' => Ok(byte - b'a' + 26),
			b'0'..=b'9' => Ok(byte - b'0' + 52),
			b'+' => Ok(62),
			b'/' => Ok(63),
			_ => Err("invalid base64"),
		}
	}

	if !input.len().is_multiple_of(4) {
		return Err("invalid base64 length");
	}
	let bytes = input.as_bytes();
	let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
	for chunk in bytes.chunks(4) {
		let first = digit(chunk[0])?;
		let second = digit(chunk[1])?;
		let third = if chunk[2] == b'=' {
			0
		} else {
			digit(chunk[2])?
		};
		let fourth = if chunk[3] == b'=' {
			0
		} else {
			digit(chunk[3])?
		};
		let packed = (u32::from(first) << 18)
			| (u32::from(second) << 12)
			| (u32::from(third) << 6)
			| u32::from(fourth);
		out.push(((packed >> 16) & 0xff) as u8);
		if chunk[2] != b'=' {
			out.push(((packed >> 8) & 0xff) as u8);
		}
		if chunk[3] != b'=' {
			out.push((packed & 0xff) as u8);
		}
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use futures::executor::block_on;
	use serde_json::json;

	use super::*;
	use crate::Error;

	#[test]
	fn base64_url_round_trips() {
		let bytes = b"hello push keys!!";
		let encoded = encode_base64_url(bytes);
		assert!(!encoded.contains('+') && !encoded.contains('/') && !encoded.contains('='));
		assert_eq!(decode_base64_url(&encoded).unwrap(), bytes);
	}

	#[test]
	fn base64_covers_padding_and_alphabet_edges() {
		assert_eq!(encode_base64_url(b"a"), "YQ");
		assert_eq!(encode_base64_url(b"ab"), "YWI");
		assert_eq!(encode_base64_url(b"abc"), "YWJj");
		assert_eq!(decode_base64_url("YQ").unwrap(), b"a");
		assert_eq!(decode_base64_url("YWI").unwrap(), b"ab");
		assert_eq!(base64_decode("YQ==").unwrap(), b"a");
		assert_eq!(base64_decode("YWI=").unwrap(), b"ab");
		assert_eq!(base64_decode("YWJj").unwrap(), b"abc");
		// Standard alphabet digits '+' and '/'.
		assert_eq!(base64_decode("+/+/").unwrap(), [0xfb, 0xff, 0xbf]);
		assert_eq!(base64_decode("ab").unwrap_err(), "invalid base64 length");
		assert_eq!(base64_decode("@@@@").unwrap_err(), "invalid base64");
		assert!(matches!(
			decode_base64_url("!!!!"),
			Err(Error::Json(message)) if message.contains("invalid applicationServerKey")
		));
	}

	#[test]
	fn push_options_default_user_visible_only() {
		let options: PushOptions =
			serde_json::from_value(json!({ "application_server_key": "BNcR" })).unwrap();
		assert!(options.user_visible_only);
		assert_eq!(options.application_server_key, "BNcR");
	}

	#[cfg(not(target_arch = "wasm32"))]
	#[test]
	fn push_manager_names_its_apis_outside_a_browser() {
		let manager = PushManager { _private: () };
		let options = PushOptions::new("BNcR");
		assert_eq!(
			block_on(manager.subscribe(&options)),
			Err(Error::Unavailable("PushManager.subscribe".into()))
		);
		assert_eq!(
			block_on(manager.get_subscription()),
			Err(Error::Unavailable("PushManager.getSubscription".into()))
		);
		assert_eq!(
			block_on(manager.unsubscribe()),
			Err(Error::Unavailable("PushSubscription.unsubscribe".into()))
		);
	}
}
