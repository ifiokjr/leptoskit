use leptoskit_rpc as rpc;

/// A cron schedules a job.
#[rpc::cron("@hourly")]
pub async fn tidy(cx: &rpc::JobCx) -> rpc::Result<()> {
	let _ = cx;
	Ok(())
}

fn main() {}
