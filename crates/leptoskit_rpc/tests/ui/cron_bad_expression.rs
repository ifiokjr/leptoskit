use leptoskit_rpc as rpc;

/// Minutes run from 0 to 59.
#[rpc::cron("60 * * * *")]
#[rpc::job]
pub async fn tidy(cx: &rpc::JobCx) -> rpc::Result<()> {
	let _ = cx;
	Ok(())
}

fn main() {}
