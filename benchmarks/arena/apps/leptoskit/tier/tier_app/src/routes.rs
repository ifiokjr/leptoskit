//! Replaced by benchmarks/scripts/arena/emit/leptoskit.ts, which writes one
//! module per screen and the navigator here. This placeholder keeps the
//! committed shell buildable on its own.

use leptoskit::prelude::Element;
use leptoskit::prelude::View;

use crate::Store;

/// Everything the app renders from.
#[derive(Debug, Default)]
pub struct Model;

/// An empty screen.
pub fn screen(_app: &Store) -> Element {
	View()
}
