use leptoskit_db::Id;
use leptoskit_db::geo::Geography;
use leptoskit_db::geo::Meters;
use leptoskit_db::geo::Point;
use leptoskit_db::geo::Polygon;

/// A park, on the globe.
#[leptoskit_db::table(name = "parks")]
pub struct Park {
	pub id: Id<Self>,
	pub area: Geography<Polygon>,
}

fn main() {
	// On the globe both databases measure distances between points only.
	let _ = Park::query().filter(|park| {
		park.area
			.within_distance(&Geography(Point::new(0.0, 0.0)), Meters(10.0))
	});
}
