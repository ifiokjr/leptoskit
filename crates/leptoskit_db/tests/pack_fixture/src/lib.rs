//! A schema pack, as a library ships one: tables in its own namespace, which
//! apps read and only this crate's API writes.

use leptoskit_db::Connection;
use leptoskit_db::Id;
use leptoskit_db::Result;
use leptoskit_db::Timestamp;
use leptoskit_db::table;

/// A note the pack keeps for an app.
#[table(name = "notes", pack = "sample", public)]
pub struct Note {
	pub id: Id<Self>,
	/// What the note says.
	#[db(min_len = 1, max_len = 200)]
	pub body: String,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// Who wrote a note: a link between two of the pack's own tables.
#[table(name = "signatures", pack = "sample")]
pub struct Signature {
	pub id: Id<Self>,
	#[db(on_delete = cascade, backlink = signatures)]
	pub note_id: Id<Note>,
	pub signer: String,
}

/// The pack's API: the only way to add a note.
pub async fn add_note(connection: &impl Connection, body: &str, signer: &str) -> Result<Note> {
	let note = NewNote { body: body.into() }.insert(connection).await?;
	NewSignature {
		note_id: note.id,
		signer: signer.into(),
	}
	.insert(connection)
	.await?;
	Ok(note)
}
