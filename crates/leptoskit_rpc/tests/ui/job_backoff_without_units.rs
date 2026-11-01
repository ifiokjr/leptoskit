use leptoskit_rpc as rpc;

/// Durations need a unit.
#[rpc::job(backoff = exponential(2, 5m))]
pub async fn settle(cx: &rpc::JobCx) -> rpc::Result<()> {
	let _ = cx;
	Ok(())
}

fn main() {}
