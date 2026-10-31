//! Keyboard shortcuts from `[extension.commands]`.

use serde::Deserialize;

use crate::Result;
use crate::js;

/// A shortcut as the user has set it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Shortcut {
	/// The command's name.
	pub name: String,
	/// The keys, or an empty string when the user removed them.
	#[serde(default)]
	pub shortcut: String,
}

/// Every shortcut, with the keys the user chose.
///
/// # Errors
///
/// Outside an extension.
pub async fn all() -> Result<Vec<Shortcut>> {
	js::call("commands.getAll", &[]).await
}
