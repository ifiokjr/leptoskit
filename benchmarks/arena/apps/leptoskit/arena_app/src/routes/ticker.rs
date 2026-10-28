//! The `ticker` wall: 20 rows of 15 tiles.

use leptoskit::prelude::Element;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;

/// Tiles per row.
pub const COLUMNS: u64 = 15;
/// Rows of tiles.
pub const ROWS: u64 = 20;

/// The wall at `frame`: tile `i` shows `frame + i`.
pub fn grid(frame: u64) -> Element {
	View().class("flex-1").children((0..ROWS).map(|row| {
		View()
			.class("h-8 flex-row")
			.children((0..COLUMNS).map(|column| {
				let tile = row * COLUMNS + column;

				View()
					.class("flex-1 items-center justify-center")
					.child(Text((frame + tile).to_string()).class("text-[10px]"))
			}))
	}))
}
