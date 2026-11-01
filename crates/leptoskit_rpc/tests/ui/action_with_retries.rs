use leptoskit_rpc as rpc;

/// Actions may reach the outside world, so they never run again by
/// themselves.
#[rpc::action(retries = 3)]
pub async fn summarize(cx: &rpc::ActionCx) -> rpc::Result<String> {
	let _ = cx;
	Ok(String::new())
}

fn main() {}
