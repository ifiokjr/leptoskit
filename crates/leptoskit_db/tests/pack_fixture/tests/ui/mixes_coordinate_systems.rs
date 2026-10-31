use leptoskit_db::Id;
use leptoskit_db::geo::Geo;
use leptoskit_db::geo::Point;
use leptoskit_db::geo::Srid;

/// A plot on the British National Grid.
#[leptoskit_db::table(name = "plots")]
pub struct Plot {
	pub id: Id<Self>,
	pub corner: Geo<Point, Srid<27700>>,
}

fn main() {
	// A point in Web Mercator can't be compared with the grid.
	let _ = Plot::query().filter(|plot| {
		plot.corner
			.intersects(&Geo::<Point, Srid<3857>>::new(Point::new(0.0, 0.0)))
	});
}
