use leptoskit_db::Id;
use leptoskit_db::table;
use leptoskit_rpc as rpc;

#[table(name = "notes")]
pub struct Note {
	pub id: Id<Self>,
	pub body: String,
}

/// Tries to write while it reads.
#[rpc::query]
pub async fn forge(cx: &rpc::QueryCx) -> rpc::Result<()> {
	NewNote { body: "forged".into() }.insert(cx.db()).await?;
	Ok(())
}

fn main() {}
