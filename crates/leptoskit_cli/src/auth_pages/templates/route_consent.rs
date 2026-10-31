//! `/consent`, written by `leptoskit generate auth --idp`. `/oauth/authorize`
//! sends people here when an app asks for something they haven't allowed.

use leptoskit::prelude::*;
use leptoskit_router::Route;

/// `?request=` names what the app asked for.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Consent {
	/// The request's handle, which works once.
	pub request: Option<String>,
}

/// Asks the signed-in person whether to let the app in.
pub fn page(route: &Consent) -> Element {
	let screens = crate::auth::screens();

	if !screens.sign_in.is_signed_in() {
		return screens.sign_in.screen();
	}

	if let Some(request) = &route.request {
		screens.consent.open(request);
	}

	screens.consent.screen()
}
