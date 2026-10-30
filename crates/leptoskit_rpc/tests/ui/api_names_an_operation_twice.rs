use leptoskit_rpc as rpc;
use leptoskit_rpc::api;

/// Named twice.
#[api(name = "recent")]
#[rpc::query(name = "latest")]
pub async fn messages(cx: &rpc::QueryCx) -> rpc::Result<u32> {
	let _ = cx;
	Ok(1)
}

fn main() {}
