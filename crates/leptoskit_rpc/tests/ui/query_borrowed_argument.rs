use leptoskit_rpc as rpc;

/// Borrows its argument.
#[rpc::query]
pub async fn search(cx: &rpc::QueryCx, text: &str) -> rpc::Result<u32> {
	let _ = (cx, text);
	Ok(1)
}

fn main() {}
