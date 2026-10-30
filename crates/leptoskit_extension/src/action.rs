//! The toolbar button: its badge, title, and popup.

use serde_json::Value;
use serde_json::json;

use crate::Result;
use crate::js;

/// Shows `text` on the toolbar button, for one tab or every tab; an empty
/// string clears it. Keep it to about four characters.
///
/// # Errors
///
/// Outside an extension.
pub async fn set_badge(text: &str, tab: Option<i64>) -> Result<()> {
	let mut details = json!({ "text": text });
	if let Some(tab) = tab {
		details["tabId"] = json!(tab);
	}
	js::call::<Value>("action.setBadgeText", &[&details])
		.await
		.map(drop)
}

/// The badge's background color, as CSS, such as `#b45309`.
///
/// # Errors
///
/// Outside an extension.
pub async fn set_badge_color(color: &str) -> Result<()> {
	js::call::<Value>(
		"action.setBadgeBackgroundColor",
		&[&json!({ "color": color })],
	)
	.await
	.map(drop)
}

/// The button's tooltip.
///
/// # Errors
///
/// Outside an extension.
pub async fn set_title(title: &str) -> Result<()> {
	js::call::<Value>("action.setTitle", &[&json!({ "title": title })])
		.await
		.map(drop)
}

/// Opens the popup as if the user had pressed the button. Browsers allow it
/// only right after a user gesture, such as a keyboard shortcut.
///
/// # Errors
///
/// When the browser can't, or won't, open it now.
pub async fn open_popup() -> Result<()> {
	js::call::<Value>("action.openPopup", &[]).await.map(drop)
}
