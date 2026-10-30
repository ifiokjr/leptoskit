use leptoskit_db_pack_fixture::Note;

async fn link(db: &leptoskit_db::Db, note: Note, other: Note) {
	let _ = note.related().add(db, other.id, ()).await;
	let _ = note.related().remove(db, other.id).await;
}

fn main() {}
