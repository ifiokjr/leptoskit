//! The extension itself: messages between its surfaces, its ID, and the
//! addresses of its files.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::Error;
use crate::Result;
use crate::js;

/// The server `[extension].server` names, such as
/// `https://api.example.com`, without a trailing slash. `leptoskit build`
/// compiles it in.
#[must_use]
pub fn server() -> Option<&'static str> {
	option_env!("LEPTOSKIT_EXTENSION_SERVER").map(|server| server.trim_end_matches('/'))
}

/// Sends `message` to the background worker and waits for its reply, which
/// the worker's [`on_message`](crate::Background::on_message) handler
/// returns. Works from every page, overlay, and the worker's own tasks.
///
/// # Errors
///
/// When no worker answers, or its handler failed, with its reason.
pub async fn send<M: Serialize, R: DeserializeOwned>(message: &M) -> Result<R> {
	let reply: Value = js::call("runtime.sendMessage", &[message]).await?;

	if let Some(error) = reply.get("__leptoskit_error").and_then(Value::as_str) {
		return Err(Error::Browser(error.to_owned()));
	}

	serde_json::from_value(reply).map_err(|error| Error::Json(error.to_string()))
}

/// The extension's ID: assigned by the store on Chromium, the add-on ID on
/// Firefox, and generated per install on Safari.
///
/// # Errors
///
/// Outside an extension.
pub fn id() -> Result<String> {
	#[cfg(target_arch = "wasm32")]
	{
		let runtime = js::object("runtime")?;

		return js_sys::Reflect::get(&runtime, &"id".into())
			.ok()
			.and_then(|id| id.as_string())
			.ok_or_else(|| Error::Unavailable("runtime.id".into()));
	}
	#[allow(unreachable_code)]
	Err(Error::Unavailable("runtime.id".into()))
}

/// The address of a file in the package, such as `popup.html`.
///
/// # Errors
///
/// Outside an extension.
pub async fn url(path: &str) -> Result<String> {
	js::call("runtime.getURL", &[&path]).await
}

/// Opens the options page.
///
/// # Errors
///
/// When the extension has none.
pub async fn open_options() -> Result<()> {
	js::call::<Value>("runtime.openOptionsPage", &[])
		.await
		.map(drop)
}

/// Reloads the whole extension: every page closes, and the worker starts
/// again.
///
/// # Errors
///
/// Outside an extension.
pub async fn reload() -> Result<()> {
	js::call::<Value>("runtime.reload", &[]).await.map(drop)
}
