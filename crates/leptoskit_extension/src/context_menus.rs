//! Items in the browser's context menu. Create them when the extension is
//! installed; the browser keeps them.

use serde_json::Value;
use serde_json::json;

use crate::Result;
use crate::js;

/// Where an item appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
	/// On selected text. `%s` in the title becomes the selection.
	Selection,
	/// Anywhere on a page.
	Page,
	/// On a link.
	Link,
	/// On an image.
	Image,
}

impl Context {
	const fn name(self) -> &'static str {
		match self {
			Self::Selection => "selection",
			Self::Page => "page",
			Self::Link => "link",
			Self::Image => "image",
		}
	}
}

/// Adds an item `id` titled `title`, shown in `contexts`. Choosing it runs
/// the worker's [`on_context_menu`](crate::Background::on_context_menu).
///
/// # Errors
///
/// Without the `contextMenus` permission, or when `id` already exists.
pub async fn create(id: &str, title: &str, contexts: &[Context]) -> Result<()> {
	let contexts: Vec<&str> = contexts.iter().map(|context| context.name()).collect();
	js::call::<Value>(
		"contextMenus.create",
		&[&json!({ "id": id, "title": title, "contexts": contexts })],
	)
	.await
	.map(drop)
}

/// Removes every item the extension added.
///
/// # Errors
///
/// Without the `contextMenus` permission.
pub async fn remove_all() -> Result<()> {
	js::call::<Value>("contextMenus.removeAll", &[])
		.await
		.map(drop)
}
