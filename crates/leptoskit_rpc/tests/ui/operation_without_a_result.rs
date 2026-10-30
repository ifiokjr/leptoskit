use leptoskit_rpc as rpc;

/// Returns no `rpc::Result`.
#[rpc::query]
pub async fn latest(cx: &rpc::QueryCx) -> u32 {
	let _ = cx;
	1
}

fn main() {}
