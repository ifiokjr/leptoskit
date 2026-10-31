//! `/device`, written by `leptoskit generate auth --idp`: where people enter
//! the code a CLI or a TV shows them.

use leptoskit::prelude::*;
use leptoskit_router::Route;

/// `?user_code=` comes with the link a device shows, so the code needn't be
/// typed.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Device {
	/// The code the device shows, such as `WDJB-MJHT`.
	pub user_code: Option<String>,
}

/// Lets the signed-in person approve a device.
pub fn page(route: &Device) -> Element {
	let screens = crate::auth::screens();

	if !screens.sign_in.is_signed_in() {
		return screens.sign_in.screen();
	}

	if route.user_code.is_some() {
		screens.device_code.open(route.user_code.as_deref());
	}

	screens.device_code.screen()
}
