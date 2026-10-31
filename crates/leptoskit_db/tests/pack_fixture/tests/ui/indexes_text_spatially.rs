use leptoskit_db::Id;

/// A name can't be indexed spatially: only shapes have a place.
#[leptoskit_db::table(name = "towns")]
pub struct Town {
	pub id: Id<Self>,
	#[db(geo(index))]
	pub name: String,
}

fn main() {}
