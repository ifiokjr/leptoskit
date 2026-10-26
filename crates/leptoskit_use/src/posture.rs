pub use leptoskit_plugin_posture::Axis;
pub use leptoskit_plugin_posture::Fold;
pub use leptoskit_plugin_posture::Hinge;
pub use leptoskit_plugin_posture::Posture;
pub use leptoskit_plugin_posture::Region;
use reactive_graph::computed::Memo;
use reactive_graph::prelude::*;
use reactive_graph::wrappers::read::Signal;

/// Reactive fold state derived from a host-supplied [`Posture`].
///
/// Every field is memoized: a hinge angle that changes while the device stays
/// half open does not notify readers of `is_half_open` or `panes`.
#[derive(Debug, Clone, Copy)]
pub struct UsePostureReturn {
	/// The device's overall fold.
	pub fold: Signal<Fold>,
	/// Folded shut, with the app on the cover screen.
	pub is_closed: Signal<bool>,
	/// Partly open.
	pub is_half_open: Signal<bool>,
	/// Open flat.
	pub is_open: Signal<bool>,
	/// Half open with a side-to-side hinge, resting like a laptop.
	pub is_tabletop: Signal<bool>,
	/// Half open with a top-to-bottom hinge, held like a book.
	pub is_book: Signal<bool>,
	/// The first hinge's angle in whole degrees, when the platform measures
	/// it: 0 shut, 180 flat.
	pub angle: Signal<Option<u16>>,
	/// The window's regions on either side of each crossing hinge, as
	/// fractions of the window. One full-window pane when nothing crosses it.
	pub panes: Signal<Vec<Region>>,
}

/// Derive fold queries from the host's posture signal.
///
/// Browsers feed the signal from `leptoskit_plugin_posture::web::watch`; SSR
/// and tests can supply any posture. No device API is accessed here. Native
/// Wasm guests cannot load the reactive runtime yet, so they read the
/// plugin's [`Posture`] directly and call its methods of the same names.
pub fn use_posture(posture: impl Into<Signal<Posture>>) -> UsePostureReturn {
	let posture = posture.into();
	let fold = Memo::new(move |_| posture.with(|posture| posture.fold));
	let is = move |expected: Fold| Signal::from(Memo::new(move |_| fold.get() == expected));
	UsePostureReturn {
		fold: fold.into(),
		is_closed: is(Fold::Closed),
		is_half_open: is(Fold::HalfOpen),
		is_open: is(Fold::Open),
		is_tabletop: Memo::new(move |_| posture.with(Posture::is_tabletop)).into(),
		is_book: Memo::new(move |_| posture.with(Posture::is_book)).into(),
		angle: Memo::new(move |_| {
			posture.with(|posture| posture.hinges.first().and_then(|hinge| hinge.angle))
		})
		.into(),
		panes: Memo::new(move |_| posture.with(Posture::panes)).into(),
	}
}
