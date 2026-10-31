use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Tracked;
use leptoskit_db::table;

#[table(name = "notes")]
pub struct Note {
	pub id: Id<Self>,
	pub body: String,
}

async fn write(db: Db) {
	let tracked = Tracked::new(db);
	let mut note = Note::query().one(&tracked).await.unwrap();
	let _ = NewNote { body: "forged".into() }.insert(&tracked).await;
	let _ = note.save(&tracked).await;
	let _ = Note::query().delete(&tracked).await;
}

fn main() {}
