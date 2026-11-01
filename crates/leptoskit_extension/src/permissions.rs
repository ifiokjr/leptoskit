//! Asking for more access at run time, such as one more site.

use serde_json::json;

use crate::Result;
use crate::js;

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

/// Whether the extension has host permission for every one of `origins`.
///
/// # Errors
///
/// Outside an extension.
pub async fn contains(origins: &[String]) -> Result<bool> {
	js::call("permissions.contains", &[&json!({ "origins": origins })]).await
}

/// Gives up host permission for `origins`.
///
/// # Errors
///
/// When a pattern was granted at install, which can't be removed.
pub async fn remove(origins: &[String]) -> Result<bool> {
	js::call("permissions.remove", &[&json!({ "origins": origins })]).await
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
