//! Operations as an app's shared crate declares them. A guest build keeps
//! only the argument structs and the client; the bodies, and every database
//! type they name, exist only in the server build.
//!
//! The body of [`SERVER_ONLY`]'s query embeds that marker, so a guest build's
//! Wasm can be searched for it: it must not be there.

use leptoskit_rpc as rpc;

/// Text only the server build's bodies contain.
pub const SERVER_ONLY: &str = "server-only body of the guest fixture";

/// A greeting, from the server.
#[rpc::query]
pub async fn greeting(cx: &rpc::QueryCx, name: String) -> rpc::Result<String> {
	let _ = cx;
	Ok(format!(
		"{name}: {}",
		"server-only body of the guest fixture"
	))
}

/// Counts something, on the server.
#[rpc::mutation(retries = 2)]
pub async fn count(cx: &rpc::MutationCx, by: u32) -> rpc::Result<u32> {
	let _ = cx;
	Ok(by + 1)
}

/// What a guest calls: the arguments go to the server, and the answer
/// comes back.
pub async fn greet(name: String) -> Result<String, rpc::FnError> {
	rpc::client::call(Greeting { name }).await
}

/// Keeps the client code in the Wasm, so the build is searched with it in.
#[allow(
	unsafe_code,
	reason = "an export keeps the client in the Wasm being searched"
)]
#[unsafe(no_mangle)]
pub extern "C" fn guest_fixture_start() {
	let future = greet("ada".to_owned());
	drop(future);
	drop(rpc::client::call(Count { by: 1 }));
}

#[cfg(all(test, feature = "server"))]
mod tests {
	use super::*;

	#[tokio::test]
	async fn the_server_build_runs_the_bodies() {
		let db = leptoskit_db::testing::sqlite().await;
		assert_eq!(
			rpc::call(
				&db,
				Greeting {
					name: "ada".to_owned()
				}
			)
			.await
			.unwrap(),
			format!("ada: {SERVER_ONLY}")
		);
		assert_eq!(rpc::call(&db, Count { by: 1 }).await.unwrap(), 2);
	}
}
