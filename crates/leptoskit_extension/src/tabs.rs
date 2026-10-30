//! Tabs: the one the user is looking at, opening pages, and messaging a
//! tab's content scripts.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;

use crate::Result;
use crate::js;

/// A browser tab. The address and title are only present with the `tabs`
/// permission, host permission for the page, or `activeTab` after the user
/// invoked the extension on it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tab {
	/// The tab's ID; missing for a few browser tabs.
	#[serde(default)]
	pub id: Option<i64>,
	/// The window it's in.
	#[serde(default)]
	pub window_id: Option<i64>,
	/// The page's address.
	#[serde(default)]
	pub url: Option<String>,
	/// The page's title.
	#[serde(default)]
	pub title: Option<String>,
	/// Whether it's the window's selected tab.
	#[serde(default)]
	pub active: bool,
}

/// The tab the user is looking at, in the window they last used.
///
/// # Errors
///
/// Outside an extension.
pub async fn active() -> Result<Option<Tab>> {
	let tabs: Vec<Tab> = js::call(
		"tabs.query",
		&[&json!({ "active": true, "lastFocusedWindow": true })],
	)
	.await?;
	Ok(tabs.into_iter().next())
}

/// Opens `url` in a new tab, and returns it.
///
/// # Errors
///
/// When the browser refuses the address.
pub async fn open(url: &str) -> Result<Tab> {
	js::call("tabs.create", &[&json!({ "url": url })]).await
}

/// Sends a message to the content scripts in a tab, and waits for the
/// reply.
///
/// # Errors
///
/// When nothing in the tab listens.
pub async fn send<M: Serialize, R: serde::de::DeserializeOwned>(
	tab: i64,
	message: &M,
) -> Result<R> {
	let reply: Value = js::call("tabs.sendMessage", &[&tab, message]).await?;
	serde_json::from_value(reply).map_err(|error| crate::Error::Json(error.to_string()))
}
