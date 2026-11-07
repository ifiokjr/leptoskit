//! Linux presentation: tray snapshots only.
//!
//! Standalone application or window menu bars are unavailable on Linux in this
//! crate. GTK 3 is deliberately off (muda's snapshot backend feeds KSNI trays
//! without deprecated GTK 3). A later GTK 4 host can own its own window menu;
//! until then [`crate::supports_menu_bar`] is `false` and
//! [`attach_menu_bar`] returns [`Error::Unavailable`].
//!
//! Tray hosts still consume a [`crate::DesktopMenu`] through
//! `leptoskit_plugin_tray::TrayHost::set_menu`.

use crate::Error;

/// Explicit no-op for hosts that share one attach call across desktop targets.
///
/// Always fails with [`Error::Unavailable`]. Prefer gating on
/// [`crate::supports_menu_bar`] so callers skip the attempt.
pub fn attach_menu_bar() -> Result<(), Error> {
	Err(Error::Unavailable)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn menu_bars_are_explicitly_unavailable() {
		assert_eq!(attach_menu_bar(), Err(Error::Unavailable));
		assert!(!crate::supports_menu_bar());
	}
}
