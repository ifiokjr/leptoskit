use leptoskit_rpc as rpc;

/// Settles a payment.
#[rpc::job(on_failure = gave_up)]
pub async fn settle(cx: &rpc::JobCx, payment: u64) -> rpc::Result<()> {
	let _ = (cx, payment);
	Ok(())
}

/// Takes a payment, not the failed job.
#[rpc::mutation]
pub async fn gave_up(cx: &rpc::MutationCx, payment: u64) -> rpc::Result<()> {
	let _ = (cx, payment);
	Ok(())
}

fn main() {}
