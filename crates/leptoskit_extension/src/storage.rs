//! Extension storage, shared by every surface and the worker. `local`
//! persists on this device, `session` lasts until the browser quits, and
//! `sync` follows the user's browser account where the browser supports it.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Map;
use serde_json::Value;

use crate::Result;
use crate::js;

/// One storage area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
	/// On this device, until the extension is removed.
	Local,
	/// Until the browser quits. Unavailable in some browsers.
	Session,
	/// Synced with the user's browser account, with small quotas.
	Sync,
}

impl Area {
	const fn path(self) -> &'static str {
		match self {
			Self::Local => "storage.local",
			Self::Session => "storage.session",
			Self::Sync => "storage.sync",
		}
	}

	/// The value under `key`, or `None` when nothing is stored there.
	///
	/// # Errors
	///
	/// When the area is unavailable, or the value isn't a `T`.
	pub async fn get<T: DeserializeOwned>(self, key: &str) -> Result<Option<T>> {
		let mut values: Map<String, Value> =
			js::call(&format!("{}.get", self.path()), &[&key]).await?;
		values
			.remove(key)
			.map(|value| {
				serde_json::from_value(value).map_err(|error| crate::Error::Json(error.to_string()))
			})
			.transpose()
	}

	/// Stores `value` under `key`.
	///
	/// # Errors
	///
	/// When the area is unavailable, or over its quota.
	pub async fn set<T: Serialize>(self, key: &str, value: &T) -> Result<()> {
		let mut values = Map::new();
		values.insert(
			key.to_owned(),
			serde_json::to_value(value).map_err(|error| crate::Error::Json(error.to_string()))?,
		);
		js::call::<Value>(&format!("{}.set", self.path()), &[&values])
			.await
			.map(drop)
	}

	/// Removes what is stored under `key`.
	///
	/// # Errors
	///
	/// When the area is unavailable.
	pub async fn remove(self, key: &str) -> Result<()> {
		js::call::<Value>(&format!("{}.remove", self.path()), &[&key])
			.await
			.map(drop)
	}

	/// Removes every key in this area.
	///
	/// # Errors
	///
	/// When the area is unavailable.
	pub async fn clear(self) -> Result<()> {
		js::call::<Value>(&format!("{}.clear", self.path()), &[])
			.await
			.map(drop)
	}

	/// Every key stored in this area.
	///
	/// # Errors
	///
	/// When the area is unavailable.
	pub async fn keys(self) -> Result<Vec<String>> {
		let values: Map<String, Value> =
			js::call(&format!("{}.get", self.path()), &[&Value::Null]).await?;
		Ok(values.into_iter().map(|(key, _)| key).collect())
	}

	/// Approximate bytes used by `key`, or by the whole area when `key` is
	/// `None`. Chromium and Firefox support this; Safari may not.
	///
	/// # Errors
	///
	/// When the area is unavailable, or the browser has no quota API.
	pub async fn get_bytes_in_use(self, key: Option<&str>) -> Result<u64> {
		let path = format!("{}.getBytesInUse", self.path());
		let bytes: f64 = match key {
			Some(key) => js::call(&path, &[&key]).await?,
			None => js::call(&path, &[]).await?,
		};
		Ok(bytes as u64)
	}
}

/// On this device: the area to use unless there's a reason not to.
pub const LOCAL: Area = Area::Local;
/// Until the browser quits.
pub const SESSION: Area = Area::Session;
/// Synced with the user's browser account.
pub const SYNC: Area = Area::Sync;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
	use futures::executor::block_on;
	use serde_json::json;

	use super::*;

	#[test]
	fn keys_and_bytes_map_browser_replies() {
		let keys = js::with_call_override(json!({ "a": 1, "b": true }), || {
			block_on(Area::Local.keys()).expect("keys")
		});
		assert_eq!(keys.len(), 2);
		assert!(keys.contains(&"a".to_owned()));
		assert!(keys.contains(&"b".to_owned()));

		let bytes = js::with_call_override(json!(42.0), || {
			block_on(Area::Local.get_bytes_in_use(Some("a"))).expect("bytes")
		});
		assert_eq!(bytes, 42);
		let bytes = js::with_call_override(json!(7.0), || {
			block_on(Area::Sync.get_bytes_in_use(None)).expect("area bytes")
		});
		assert_eq!(bytes, 7);
	}
}

/// Calls `changed` with the key and new value (or `None`, when removed)
/// whenever a value in `area` changes, from any surface or the worker, until
/// the returned listener is dropped.
///
/// # Errors
///
/// Outside an extension.
pub fn on_changed(
	area: Area,
	changed: impl Fn(&str, Option<Value>) + 'static,
) -> Result<crate::Listener> {
	#[cfg(target_arch = "wasm32")]
	{
		let name = match area {
			Area::Local => "local",
			Area::Session => "session",
			Area::Sync => "sync",
		};
		return js::listen("storage.onChanged", move |changes, changed_area, _| {
			if changed_area.as_string().as_deref() != Some(name) {
				return wasm_bindgen::JsValue::UNDEFINED;
			}
			if let Ok(changes) = js::from_js::<Map<String, Value>>(&changes) {
				for (key, change) in changes {
					changed(&key, change.get("newValue").cloned());
				}
			}

			wasm_bindgen::JsValue::UNDEFINED
		});
	}
	#[allow(unreachable_code)]
	{
		let _ = (area, changed);
		Err(crate::Error::Unavailable("storage.onChanged".into()))
	}
}
