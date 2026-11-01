use leptoskit_db::Id;
use leptoskit_db::vector::Vector;

/// A passage with a 3-dimension embedding.
#[leptoskit_db::table(name = "passages")]
pub struct Passage {
	pub id: Id<Self>,
	#[db(vector(distance = cosine))]
	pub embedding: Option<Vector<3>>,
}

fn main() {
	// An embedding from a model with 4 dimensions can't be compared.
	let _ = Passage::query().nearest(|passage| passage.embedding, &Vector::new([0.0; 4]));
}
