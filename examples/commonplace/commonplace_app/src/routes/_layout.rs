//! The frame around every screen, and the gate in front of it: someone who
//! isn't signed in sees the sign-in form instead.

use leptoskit::prelude::*;

use crate::routes::Library;
use crate::routes::Navigation;
use crate::store::Session;
use crate::store::Store;
use crate::ui;

pub fn layout(children: Children, navigation: &Navigation) -> Element {
	let store = Store::current();
	let session = store.with(|state| state.session);
	let body = match session {
		Session::Checking => ui::loading("your library"),
		Session::SignedOut => crate::sign_in::form(&store),
		Session::SignedIn => View().class("gap-4").children(children),
	};
	let header = View()
		.class(
			"flex-row items-center justify-between border-b border-stone-200 px-6 py-3 \
			 dark:border-stone-800",
		)
		.child(
			navigation
				.link(Library, "Commonplace, your library")
				.child(Text("Commonplace").class("text-xl font-semibold text-amber-700")),
		);
	let header = if session == Session::SignedIn {
		header.child(ui::secondary("Sign out", move || {
			Store::current().sign_out()
		}))
	} else {
		header
	};

	ScrollView().class(ui::PAGE).child(header).child(
		View()
			.class("w-full max-w-3xl self-center gap-4 p-6")
			.child(match store.with(|state| state.notice.clone()) {
				Some(notice) => ui::problem(&notice),
				None => View(),
			})
			.child(body),
	)
}
