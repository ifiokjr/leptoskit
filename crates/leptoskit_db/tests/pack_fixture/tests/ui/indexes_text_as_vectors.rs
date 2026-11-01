use leptoskit_db::Id;

/// A note.
#[leptoskit_db::table(name = "notes")]
pub struct Note {
	pub id: Id<Self>,
	#[db(vector(distance = l2))]
	pub body: String,
}

fn main() {}
