//! The frame around every screen, and the gate in front of it: someone who
//! isn't signed in sees the sign-in form instead.

use leptoskit::prelude::*;

use crate::routes::Library;
use crate::routes::Navigation;
use crate::routes::Route;
use crate::store::Session;
use crate::store::Store;
use crate::ui;

#[component]
pub fn Layout(children: Children, navigation: &Navigation) -> Element {
	let store = Store::current();
	let session = store.with(|state| state.session);
	// A frame on someone else's page has no room for the app's chrome.
	if matches!(
		navigation.current(),
		Route::OverlayClipper | Route::OverlayHighlights
	) {
		return view! { <View class=ui::PAGE>{children}</View> };
	}
	let body = match session {
		Session::Checking => view! { <ui::Loading what="your library" /> },
		Session::SignedOut => view! { <crate::sign_in::Form store=&store /> },
		Session::SignedIn => view! { <View class="gap-4">{children}</View> },
	};
	let header = view! {
		<View class="flex-row items-center justify-between border-b border-stone-200 px-6 py-3 \
			 dark:border-stone-800">
			{navigation
				.link(Library, "Commonplace, your library")
				.child(view! {
					<Text class="text-xl font-semibold text-amber-700">"Commonplace"</Text>
				})}
		</View>
	};
	let header = if session == Session::SignedIn {
		header.child(view! {
			<ui::Secondary label="Sign out" on_press=move || Store::current().sign_out() />
		})
	} else {
		header
	};

	view! {
		<ScrollView class=ui::PAGE>
			{header}
			<View class="w-full max-w-3xl self-center gap-4 p-6">
				{match store.with(|state| state.notice.clone()) {
					Some(notice) => view! { <ui::Problem message=&notice /> },
					None => view! { <View /> },
				}}
				{body}
			</View>
		</ScrollView>
	}
}
