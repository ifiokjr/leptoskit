//! Asking for more access at run time, such as one more site.

use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

use crate::Result;
use crate::js;

/// The permissions the extension currently holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permissions {
	/// Named permissions such as `storage` and `alarms`.
	#[serde(default)]
	pub permissions: Vec<String>,
	/// Host match patterns such as `https://example.org/*`.
	#[serde(default)]
	pub origins: Vec<String>,
}

/// Asks the user to grant host permission for `origins`, such as
/// `https://example.org/*`. Browsers only show the prompt right after a user
/// gesture, such as pressing a button. Returns whether they granted it.
///
/// # Errors
///
/// When a pattern isn't in the manifest's `optional_host_permissions`.
pub async fn request(origins: &[String]) -> Result<bool> {
	js::call("permissions.request", &[&json!({ "origins": origins })]).await
}

/// Asks the user to grant named `permissions` and optional `origins`.
///
/// # Errors
///
/// When a permission isn't optional in the manifest.
pub async fn request_permissions(permissions: &Permissions) -> Result<bool> {
	js::call("permissions.request", &[permissions]).await
}

/// Whether the extension has host permission for every one of `origins`.
///
/// # Errors
///
/// Outside an extension.
pub async fn contains(origins: &[String]) -> Result<bool> {
	js::call("permissions.contains", &[&json!({ "origins": origins })]).await
}

/// Whether the extension holds every named permission and origin in
/// `permissions`.
///
/// # Errors
///
/// Outside an extension.
pub async fn contains_permissions(permissions: &Permissions) -> Result<bool> {
	js::call("permissions.contains", &[permissions]).await
}

/// Gives up host permission for `origins`.
///
/// # Errors
///
/// When a pattern was granted at install, which can't be removed.
pub async fn remove(origins: &[String]) -> Result<bool> {
	js::call("permissions.remove", &[&json!({ "origins": origins })]).await
}

/// Gives up the named permissions and origins in `permissions`.
///
/// # Errors
///
/// When a permission was granted at install, which can't be removed.
pub async fn remove_permissions(permissions: &Permissions) -> Result<bool> {
	js::call("permissions.remove", &[permissions]).await
}

/// Every permission the extension currently holds.
///
/// # Errors
///
/// Outside an extension.
pub async fn get_all() -> Result<Permissions> {
	js::call("permissions.getAll", &[]).await
}

/// Calls `changed` whenever the user grants more optional permissions, until
/// the returned listener is dropped.
///
/// # Errors
///
/// Outside an extension.
pub fn on_added(changed: impl Fn(Permissions) + 'static) -> Result<crate::Listener> {
	#[cfg(target_arch = "wasm32")]
	{
		return js::listen("permissions.onAdded", move |permissions, _, _| {
			if let Ok(permissions) = js::from_js::<Permissions>(&permissions) {
				changed(permissions);
			}
			wasm_bindgen::JsValue::UNDEFINED
		});
	}
	#[allow(unreachable_code)]
	{
		let _ = changed;
		Err(crate::Error::Unavailable("permissions.onAdded".into()))
	}
}

/// Calls `changed` whenever optional permissions are removed, until the
/// returned listener is dropped.
///
/// # Errors
///
/// Outside an extension.
pub fn on_removed(changed: impl Fn(Permissions) + 'static) -> Result<crate::Listener> {
	#[cfg(target_arch = "wasm32")]
	{
		return js::listen("permissions.onRemoved", move |permissions, _, _| {
			if let Ok(permissions) = js::from_js::<Permissions>(&permissions) {
				changed(permissions);
			}
			wasm_bindgen::JsValue::UNDEFINED
		});
	}
	#[allow(unreachable_code)]
	{
		let _ = changed;
		Err(crate::Error::Unavailable("permissions.onRemoved".into()))
	}
}

/// The match pattern for a page's whole site, such as
/// `https://example.org/*`, from any address on it.
#[must_use]
pub fn site_pattern(url: &str) -> Option<String> {
	let (scheme, rest) = url.split_once("://")?;

	if scheme != "http" && scheme != "https" {
		return None;
	}

	let host = rest.split(['/', '?', '#']).next()?;
	let host = host.rsplit('@').next()?;
	(!host.is_empty()).then(|| format!("{scheme}://{host}/*"))
}

#[cfg(test)]
mod tests {
	#[test]
	fn site_patterns() {
		assert_eq!(
			super::site_pattern("https://example.org/a/b?c#d").as_deref(),
			Some("https://example.org/*")
		);
		assert_eq!(super::site_pattern("chrome://settings"), None);
	}
}
