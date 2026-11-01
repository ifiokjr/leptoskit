use leptoskit_rpc as rpc;

/// Nothing gives a cron job its arguments.
#[rpc::cron("@daily")]
#[rpc::job]
pub async fn remind(cx: &rpc::JobCx, user: u64) -> rpc::Result<()> {
	let _ = (cx, user);
	Ok(())
}

fn main() {}
