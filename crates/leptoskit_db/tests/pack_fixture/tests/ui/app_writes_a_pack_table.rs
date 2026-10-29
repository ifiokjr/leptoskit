use leptoskit_db_pack_fixture::NewNote;
use leptoskit_db_pack_fixture::Note;

async fn write(db: &leptoskit_db::Db, mut note: Note) {
	let _ = NewNote { body: "forged".into() }.insert(db).await;
	let _ = note.save(db).await;
	let _ = Note::update();
}

fn main() {}
