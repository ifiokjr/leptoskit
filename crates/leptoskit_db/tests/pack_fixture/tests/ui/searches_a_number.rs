use leptoskit_db::Id;

/// A score can't be searched: only text is split into words.
#[leptoskit_db::table(name = "scores")]
pub struct Score {
	pub id: Id<Self>,
	#[db(search)]
	pub points: i64,
}

fn main() {}
