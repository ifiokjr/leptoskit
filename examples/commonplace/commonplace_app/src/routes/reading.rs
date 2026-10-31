//! `/reading`: the extension's new tab page. Your projects, one tab away.

use leptoskit::prelude::*;

use crate::browser;
use crate::routes::Navigation;
use crate::ui;

pub fn page(navigation: &Navigation) -> Element {
	if !browser::IN_EXTENSION {
		return ui::extension_only();
	}
	View()
		.class("gap-2")
		.child(Text("A new tab, and your research where you left it.").class(ui::QUIET))
		.child(crate::routes::index::page(navigation))
}
