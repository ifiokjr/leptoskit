//! Content scripts at run time: injecting one into the tab the user acted
//! on, and registering one for the sites the user allowed.

use serde_json::Value;
use serde_json::json;

use crate::Result;
use crate::js;

/// The file the build generates for a content script.
fn file(name: &str) -> String {
	format!("content-{name}.js")
}

/// Injects the content script `name` into a tab. With `activeTab`, this
/// needs no host permission, as long as the user just invoked the extension
/// on that tab.
///
/// # Errors
///
/// When the browser refuses, such as on a browser page or without
/// permission.
pub async fn inject(tab: i64, name: &str) -> Result<()> {
	js::call::<Value>(
		"scripting.executeScript",
		&[&json!({ "target": { "tabId": tab }, "files": [file(name)] })],
	)
	.await
	.map(drop)
}

/// Registers the content script `name` to run on pages matching `matches`
/// from now on, including after the browser restarts. The extension needs
/// host permission for them; ask with
/// [`permissions::request`](crate::permissions::request).
///
/// # Errors
///
/// When it's already registered, or the browser refuses.
pub async fn register(name: &str, matches: &[String]) -> Result<()> {
	js::call::<Value>(
		"scripting.registerContentScripts",
		&[&json!([{
			"id": name,
			"js": [file(name)],
			"matches": matches,
			"runAt": "document_idle",
			"persistAcrossSessions": true,
		}])],
	)
	.await
	.map(drop)
}

/// Stops running the registered content script `name`.
///
/// # Errors
///
/// When it isn't registered.
pub async fn unregister(name: &str) -> Result<()> {
	js::call::<Value>(
		"scripting.unregisterContentScripts",
		&[&json!({ "ids": [name] })],
	)
	.await
	.map(drop)
}

/// The match patterns the content script `name` is registered for, if it
/// is.
///
/// # Errors
///
/// Outside an extension.
pub async fn registered(name: &str) -> Result<Option<Vec<String>>> {
	let scripts: Vec<Value> = js::call(
		"scripting.getRegisteredContentScripts",
		&[&json!({ "ids": [name] })],
	)
	.await?;
	Ok(scripts.into_iter().next().map(|script| {
		script["matches"]
			.as_array()
			.into_iter()
			.flatten()
			.filter_map(|pattern| pattern.as_str().map(str::to_owned))
			.collect()
	}))
}
