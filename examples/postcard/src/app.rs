//! The reactive scope the postcard lives in.

use leptoskit::prelude::*;
use leptoskit_l10n::Locale;
use reactive_graph::owner::Owner;

use crate::l10n::L10n;
use crate::screen::Screen;
use crate::trip::Trip;

/// The viewport the screen is laid out for.
pub const VIEWPORT: (f32, f32) = (1000.0, 720.0);

/// The postcard app: its translations, its trip, and the reactive owner
/// both live in.
///
/// The translations are provided once, where the app starts, so the
/// components' `use_l10n` finds them on every render and a language switch
/// survives the rebuild.
#[derive(Debug, Clone)]
pub struct PostcardApp {
	owner: Owner,
	l10n: L10n,
	trip: Trip,
}

impl PostcardApp {
	/// Starts in the supported language that best serves `preferred`, or in
	/// English when none does.
	pub fn new(preferred: &[Locale]) -> Self {
		let owner = Owner::new();
		let (l10n, trip) = owner.with(|| {
			let l10n = L10n::new(preferred);
			l10n.provide();
			(l10n, Trip::new())
		});
		Self { owner, l10n, trip }
	}

	/// The translations, for reading messages or switching language.
	///
	/// The handle's locale lives in the app's reactive owner, so keep the
	/// app alive while you use it: dropping the last clone disposes it.
	pub fn l10n(&self) -> L10n {
		self.l10n
	}

	/// The trip the postcard describes.
	pub fn trip(&self) -> Trip {
		self.trip
	}

	/// The screen for the current language and trip.
	pub fn render(&self) -> Element {
		let trip = self.trip;
		self.owner.with(|| view! { <Screen trip=trip /> })
	}
}
