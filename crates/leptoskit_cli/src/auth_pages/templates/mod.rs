//! Sign-in screens, written by `leptoskit generate auth`. They're yours:
//! restyle them, reword them, or rewrite them. Running the generator again
//! never replaces a file you've changed unless you pass `--force`.
//!
//! They're built only on `leptoskit::auth::client`, so the same screens work
//! in the browser and in native apps. Create them once when the app starts,
//! in the reactive owner that holds the app's `QueryClient`:
//!
//! ```ignore
//! leptoskit::query::QueryClient::new().provide();
//! auth::provide();
//! ```
//!
//! Then put [`Screens::gate`] in front of everything that needs someone
//! signed in, or show [`SignIn::screen`], [`Devices::screen`], and
//! [`Security::screen`] where you like.
// leptoskit: idp
//! The "Sign in with your app" screens sometimes leave this app: back to the
//! app that asked for consent, or on to `/oauth/authorize` once someone has
//! signed in for it. Say how once, after `auth::provide()`. In the browser
//! it's a full page load; a native app never shows these screens, since apps
//! signing in with yours open your site in a browser:
//!
//! ```ignore
//! auth::screens().navigate_with(|address| {
//! 	if let Some(window) = web_sys::window() {
//! 		let _ = window.location().assign(address);
//! 	}
//! });
//! ```
//!
//! [`Consent::screen`] answers an app asking to sign someone in,
//! [`DeviceCode::screen`] takes the code a CLI or a TV shows, and
//! [`ConnectedApps::screen`] and [`DeveloperApps::screen`] list the apps
//! someone connected or registered.
// leptoskit: end idp

// leptoskit: idp
mod connected_apps;
mod consent;
mod developer_apps;
mod device_code;
// leptoskit: end idp
mod devices;
mod security;
mod sign_in;

// leptoskit: idp
use std::sync::Arc;

pub use connected_apps::ConnectedApps;
pub use consent::Consent;
pub use developer_apps::DeveloperApps;
pub use device_code::DeviceCode;
// leptoskit: end idp
pub use devices::Devices;
// leptoskit: idp
use leptoskit::auth::client::use_auth;
use leptoskit::leptos::prelude::Effect;
// leptoskit: end idp
use leptoskit::leptos::prelude::Get;
use leptoskit::leptos::prelude::GetUntracked;
// leptoskit: idp
use leptoskit::leptos::prelude::GetValue;
// leptoskit: end idp
use leptoskit::leptos::prelude::RwSignal;
use leptoskit::leptos::prelude::Set;
// leptoskit: idp
use leptoskit::leptos::prelude::SetValue;
use leptoskit::leptos::prelude::StoredValue;
// leptoskit: end idp
use leptoskit::leptos::prelude::provide_context;
use leptoskit::leptos::prelude::use_context;
use leptoskit::prelude::*;
pub use security::Security;
pub use sign_in::SignIn;

/// The sign-in screens, shared through context by [`provide`].
#[derive(Clone, Copy)]
pub struct Screens {
	/// The email-and-code sign-in screen.
	pub sign_in: SignIn,
	/// The "where you're signed in" screen.
	pub devices: Devices,
	/// The security screen: passkeys, the authenticator app, and connected
	/// accounts.
	pub security: Security,
	// leptoskit: idp
	/// The answer to an app asking to sign someone in with their account.
	pub consent: Consent,
	/// Entering the code a CLI or a TV shows, to let it sign in.
	pub device_code: DeviceCode,
	/// The apps someone let sign in with their account.
	pub connected_apps: ConnectedApps,
	/// The apps someone registered in the developer portal.
	pub developer_apps: DeveloperApps,
	/// How the screens leave this app.
	leave: Leave,
	/// Where to go once someone signs in, such as back to `/oauth/authorize`.
	return_to: StoredValue<Option<String>>,
	// leptoskit: end idp
	/// Which account screen the signed-in person asked for, if any.
	showing: RwSignal<Option<Account>>,
}

/// The account screens the bar opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Account {
	Devices,
	Security,
	// leptoskit: idp
	ConnectedApps,
	// leptoskit: end idp
}

/// Creates the sign-in screens in the current reactive owner, which needs a
/// `QueryClient`, and shares them with everything rendered beneath it.
pub fn provide() -> Screens {
	let sign_in = SignIn::new();
	// leptoskit: idp
	let leave = Leave(StoredValue::new(None));
	// leptoskit: end idp
	let screens = Screens {
		sign_in,
		devices: Devices::new(move || sign_in.is_signed_in()),
		security: Security::new(move || sign_in.is_signed_in()),
		// leptoskit: idp
		consent: Consent::new(move || sign_in.is_signed_in(), leave),
		device_code: DeviceCode::new(move || sign_in.is_signed_in()),
		connected_apps: ConnectedApps::new(move || sign_in.is_signed_in()),
		developer_apps: DeveloperApps::new(move || sign_in.is_signed_in()),
		leave,
		return_to: StoredValue::new(None),
		// leptoskit: end idp
		showing: RwSignal::new(None),
	};
	provide_context(screens);
	// leptoskit: idp
	screens.continue_after_sign_in();
	// leptoskit: end idp
	screens
}

/// The screens [`provide`] created.
///
/// # Panics
///
/// When [`provide`] hasn't run in a reactive owner above this one.
pub fn screens() -> Screens {
	use_context::<Screens>().expect("call `auth::provide()` when the app starts")
}

impl Screens {
	/// Shows `home` to someone signed in, under a bar with their account's
	/// buttons, and the sign-in screen to everyone else.
	pub fn gate(&self, home: impl FnOnce() -> Element) -> Element {
		if !self.sign_in.is_signed_in() {
			return self.sign_in.screen();
		}

		let body = match self.showing.get() {
			Some(Account::Devices) => self.devices.screen(),
			Some(Account::Security) => self.security.screen(),
			// leptoskit: idp
			Some(Account::ConnectedApps) => self.connected_apps.screen(),
			// leptoskit: end idp
			None => home(),
		};

		View()
			.class("min-h-screen bg-slate-950 text-white")
			.child(self.account_bar())
			.child(View().class("flex-1 items-center").child(body))
	}

	/// Who's signed in, and the way to their devices, their security
	/// settings, and out.
	fn account_bar(&self) -> Element {
		let showing = self.showing;
		let sign_in = self.sign_in;
		let signing_out = sign_in.is_signing_out();
		let tab = move |label: &'static str, screen: Account| {
			let open = showing.get() == Some(screen);
			Button(label)
				.aria_pressed(open)
				.on_press(move || {
					showing.set(if showing.get_untracked() == Some(screen) {
						None
					} else {
						Some(screen)
					});
				})
				.class("rounded-lg px-3 py-2 text-sm font-semibold")
		};

		View()
			.class("flex-row items-center gap-3 px-6 py-3 bg-slate-900")
			.child(Text("Signed in").class("flex-1 text-sm text-slate-300"))
			.child(tab("Where you're signed in", Account::Devices))
			.child(tab("Security", Account::Security))
			// leptoskit: idp
			.child(tab("Connected apps", Account::ConnectedApps))
			// leptoskit: end idp
			.child(
				Button(if signing_out {
					"Signing out…"
				} else {
					"Sign out"
				})
				.enabled(!signing_out)
				.aria_busy(signing_out)
				.on_press(move || {
					showing.set(None);
					sign_in.sign_out();
				})
				.class("rounded-lg bg-slate-700 px-3 py-2 text-sm font-semibold"),
			)
	}
}
// leptoskit: idp

impl Screens {
	/// Says how the screens leave this app, such as back to the app that
	/// asked for consent: in the browser, `window.location.assign`.
	pub fn navigate_with(&self, navigate: impl Fn(&str) + Send + Sync + 'static) {
		self.leave.0.set_value(Some(Arc::new(navigate)));
	}

	/// Where to go once someone signs in here, from the sign-in page's
	/// `?return_to=`. Only a path on this site is kept, so a link can't send
	/// anyone elsewhere.
	pub fn return_to(&self, path: Option<&str>) {
		self.return_to
			.set_value(path.filter(|path| is_local_path(path)).map(str::to_owned));
	}

	/// Whether the sign-in page was given somewhere to go.
	pub fn is_returning(&self) -> bool {
		self.return_to.get_value().is_some()
	}

	/// Goes where [`return_to`](Self::return_to) said, once.
	pub fn continue_on(&self) {
		if let Some(path) = self.return_to.get_value() {
			self.return_to.set_value(None);
			self.leave.to(&path);
		}
	}

	/// Continues once someone signs in on this device. Someone who was
	/// already signed in isn't sent on by itself, so an app asking them to
	/// sign in again can't loop: the sign-in page offers a button instead.
	fn continue_after_sign_in(&self) {
		let screens = *self;
		let session = use_auth();

		// Effects never run while the server renders.
		Effect::new(move |seen_signed_out: Option<bool>| {
			match session.data() {
				Some(None) => true,
				Some(Some(_)) if seen_signed_out == Some(true) => {
					screens.continue_on();
					false
				}
				_ => seen_signed_out.unwrap_or(false),
			}
		});
	}
}

/// How a screen sends the browser to an address outside this app.
pub type Navigate = Arc<dyn Fn(&str) + Send + Sync>;

/// Where the screens keep the app's [`Navigate`], once
/// [`Screens::navigate_with`] gives it.
#[derive(Clone, Copy)]
pub struct Leave(StoredValue<Option<Navigate>>);

impl Leave {
	/// Sends the browser to `address`, and says whether the app said how.
	pub fn to(&self, address: &str) -> bool {
		match self.0.get_value() {
			Some(navigate) => {
				navigate(address);
				true
			}
			None => false,
		}
	}
}

/// A path on this site, which no browser reads as another site's address,
/// so a `return_to` can't send anyone elsewhere. Browsers drop tabs and line
/// breaks from addresses, so `/\t/evil.example` would be `//evil.example`:
/// whitespace and control characters are refused too.
pub fn is_local_path(path: &str) -> bool {
	path.starts_with('/')
		&& !path.starts_with("//")
		&& !path
			.chars()
			.any(|c| c == '\\' || c.is_whitespace() || c.is_control())
}

#[cfg(test)]
mod tests {
	use super::is_local_path;

	#[test]
	fn only_paths_on_this_site_are_followed() {
		assert!(is_local_path(
			"/oauth/authorize?client_id=partner&scope=openid"
		));
		assert!(!is_local_path("https://evil.example"));
		assert!(!is_local_path("//evil.example"));
		assert!(!is_local_path("/\\evil.example"));
		assert!(!is_local_path("/\t/evil.example"));
		assert!(!is_local_path("/\n/evil.example"));
		assert!(!is_local_path(""));
	}
}
// leptoskit: end idp
