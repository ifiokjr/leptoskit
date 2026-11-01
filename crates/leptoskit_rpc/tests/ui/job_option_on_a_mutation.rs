use leptoskit_rpc as rpc;

/// Queues are for jobs.
#[rpc::mutation(queue = "mail")]
pub async fn invite(cx: &rpc::MutationCx) -> rpc::Result<()> {
	let _ = cx;
	Ok(())
}

fn main() {}
