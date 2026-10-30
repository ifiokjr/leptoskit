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
//! signed in, or show [`SignIn::screen`] and [`Devices::screen`] where you
//! like.

mod devices;
mod sign_in;

pub use devices::Devices;
use leptoskit::leptos::prelude::Get;
use leptoskit::leptos::prelude::GetUntracked;
use leptoskit::leptos::prelude::RwSignal;
use leptoskit::leptos::prelude::Set;
use leptoskit::leptos::prelude::provide_context;
use leptoskit::leptos::prelude::use_context;
use leptoskit::prelude::*;
pub use sign_in::SignIn;

/// The sign-in screens, shared through context by [`provide`].
#[derive(Clone, Copy)]
pub struct Screens {
	/// The email-and-code sign-in screen.
	pub sign_in: SignIn,
	/// The "where you're signed in" screen.
	pub devices: Devices,
	/// Whether the signed-in person asked for the devices screen.
	showing_devices: RwSignal<bool>,
}

/// Creates the sign-in screens in the current reactive owner, which needs a
/// `QueryClient`, and shares them with everything rendered beneath it.
pub fn provide() -> Screens {
	let sign_in = SignIn::new();
	let screens = Screens {
		sign_in,
		devices: Devices::new(move || sign_in.is_signed_in()),
		showing_devices: RwSignal::new(false),
	};
	provide_context(screens);
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

		let body = if self.showing_devices.get() {
			self.devices.screen()
		} else {
			home()
		};

		View()
			.class("min-h-screen bg-slate-950 text-white")
			.child(self.account_bar())
			.child(View().class("flex-1 items-center").child(body))
	}

	/// Who's signed in, and the way to their devices and out.
	fn account_bar(&self) -> Element {
		let showing = self.showing_devices;
		let sign_in = self.sign_in;
		let signing_out = sign_in.is_signing_out();

		View()
			.class("flex-row items-center gap-3 px-6 py-3 bg-slate-900")
			.child(Text("Signed in").class("flex-1 text-sm text-slate-300"))
			.child(
				Button(if showing.get() {
					"Back"
				} else {
					"Where you're signed in"
				})
				.aria_pressed(showing.get())
				.on_press(move || showing.set(!showing.get_untracked()))
				.class("rounded-lg px-3 py-2 text-sm font-semibold"),
			)
			.child(
				Button(if signing_out {
					"Signing out…"
				} else {
					"Sign out"
				})
				.enabled(!signing_out)
				.aria_busy(signing_out)
				.on_press(move || {
					showing.set(false);
					sign_in.sign_out();
				})
				.class("rounded-lg bg-slate-700 px-3 py-2 text-sm font-semibold"),
			)
	}
}
