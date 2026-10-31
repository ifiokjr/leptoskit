use leptoskit_rpc as rpc;

/// Queries have no transaction to retry.
#[rpc::query(retries = 3)]
pub async fn latest(cx: &rpc::QueryCx) -> rpc::Result<u32> {
	let _ = cx;
	Ok(1)
}

fn main() {}
