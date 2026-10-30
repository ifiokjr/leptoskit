use leptoskit_rpc as rpc;

/// Neither serde nor a schema.
#[derive(Clone)]
pub struct Opaque(std::sync::Arc<()>);

/// Takes something that can't cross the wire.
#[rpc::query]
pub async fn lookup(cx: &rpc::QueryCx, key: Opaque) -> rpc::Result<u32> {
	let _ = (cx, key);
	Ok(1)
}

fn main() {}
