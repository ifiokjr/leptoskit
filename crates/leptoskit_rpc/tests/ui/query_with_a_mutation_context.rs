use leptoskit_rpc as rpc;

/// Reads with the wrong context.
#[rpc::query]
pub async fn latest(cx: &rpc::MutationCx, limit: u32) -> rpc::Result<Vec<String>> {
	let _ = (cx, limit);
	Ok(Vec::new())
}

fn main() {}
