//! The side panel: `sidePanel` on Chromium and `sidebarAction` on Firefox,
//! behind one call. Safari has neither, so it's unavailable there.

use serde_json::Value;
use serde_json::json;

use crate::Error;
use crate::Result;
use crate::js;

/// Whether this browser has a side panel.
#[must_use]
pub fn available() -> bool {
	js::has("sidePanel.open") || js::has("sidebarAction.open")
}

/// Opens the side panel in the window `window`. Browsers allow it only
/// right after a user gesture.
///
/// # Errors
///
/// [`Error::Unavailable`] in Safari; otherwise when the browser won't open
/// it now.
pub async fn open(window: i64) -> Result<()> {
	if js::has("sidePanel.open") {
		return js::call::<Value>("sidePanel.open", &[&json!({ "windowId": window })])
			.await
			.map(drop);
	}

	if js::has("sidebarAction.open") {
		return js::call::<Value>("sidebarAction.open", &[]).await.map(drop);
	}

	Err(Error::Unavailable("sidePanel".into()))
}
