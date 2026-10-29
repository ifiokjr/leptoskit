use leptos::prelude::*;
use leptoskit_rpc::api;
use serde::Deserialize;
use serde::Serialize;

/// Serializable, but with no JSON Schema to publish.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unschematic {
	pub id: u64,
}

#[api]
#[server]
pub async fn save(item: Unschematic) -> Result<(), ServerFnError> {
	let _ = item;
	Ok(())
}

fn main() {}
