use leptoskit_rpc as rpc;

/// `#[api]` must come first.
#[rpc::mutation]
#[leptoskit_rpc::api]
pub async fn save(cx: &rpc::MutationCx, text: String) -> rpc::Result<()> {
	let _ = (cx, text);
	Ok(())
}

fn main() {}
