//! `/sign-in`, written by `leptoskit generate auth --idp`.

use leptoskit::prelude::*;
use leptoskit_router::Route;

/// `?return_to=` is where to go once signed in, such as back to
/// `/oauth/authorize` for an app signing in with yours.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct SignIn {
	/// A path on this site; anything else is ignored.
	pub return_to: Option<String>,
}

/// Signs someone in with a code emailed to them, then goes on to
/// `return_to`. Someone already signed in is offered a button to go on.
pub fn page(route: &SignIn) -> Element {
	let screens = crate::auth::screens();
	screens.return_to(route.return_to.as_deref());

	if !screens.sign_in.is_signed_in() || !screens.is_returning() {
		return screens.sign_in.screen();
	}

	View()
		.class("min-h-screen items-center justify-center gap-4 p-6 bg-slate-950 text-white")
		.child(Text("You're signed in.").class("text-slate-300"))
		.child(
			Button("Continue")
				.on_press(move || screens.continue_on())
				.class("rounded-lg bg-indigo-500 px-4 py-3 font-semibold"),
		)
		.child(
			Button("Sign in as someone else")
				.on_press(move || screens.sign_in.sign_out())
				.class("rounded-lg bg-slate-700 px-4 py-3 font-semibold"),
		)
}
