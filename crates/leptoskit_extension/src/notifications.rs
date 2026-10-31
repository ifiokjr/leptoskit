//! System notifications.

use serde_json::Value;
use serde_json::json;

use crate::Result;
use crate::js;

/// Shows a notification `id` with a title and message. Showing one with an
/// existing ID replaces it.
///
/// # Errors
///
/// Without the `notifications` permission.
pub async fn show(id: &str, title: &str, message: &str) -> Result<()> {
	let icon = crate::runtime::url("assets/icon-128.png")
		.await
		.unwrap_or_default();
	js::call::<Value>(
		"notifications.create",
		&[
			&id,
			&json!({ "type": "basic", "title": title, "message": message, "iconUrl": icon }),
		],
	)
	.await
	.map(drop)
}
