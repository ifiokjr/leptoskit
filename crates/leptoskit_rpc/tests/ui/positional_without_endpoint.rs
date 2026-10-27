use leptos::prelude::*;
use leptoskit_rpc::api;

#[api]
#[server(Ping, "/api")]
pub async fn ping() -> Result<(), ServerFnError> {
	Ok(())
}

fn main() {}
