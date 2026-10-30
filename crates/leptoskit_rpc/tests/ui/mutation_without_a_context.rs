use leptoskit_rpc as rpc;

/// Writes without saying where.
#[rpc::mutation]
pub async fn reset() -> rpc::Result<()> {
	Ok(())
}

fn main() {}
