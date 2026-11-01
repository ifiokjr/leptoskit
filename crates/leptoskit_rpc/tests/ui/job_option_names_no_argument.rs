use leptoskit_rpc as rpc;

/// `serialize_by` names one of the job's arguments.
#[rpc::job(serialize_by = wallet)]
pub async fn settle(cx: &rpc::JobCx, payment: u64) -> rpc::Result<()> {
	let _ = (cx, payment);
	Ok(())
}

fn main() {}
