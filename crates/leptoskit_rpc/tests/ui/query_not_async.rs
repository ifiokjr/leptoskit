use leptoskit_rpc as rpc;

/// Not an async fn.
#[rpc::query]
pub fn latest(cx: &rpc::QueryCx) -> rpc::Result<u32> {
	let _ = cx;
	Ok(1)
}

fn main() {}
