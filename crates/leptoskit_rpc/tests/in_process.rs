//! While the server renders, `client::call` runs operations in process, as
//! the request being rendered.

#![cfg(all(feature = "sqlite", feature = "client"))]

use leptoskit_db::Context;
use leptoskit_query::BoxFuture;
use leptoskit_rpc as rpc;
use leptoskit_rpc::FnError;
use leptoskit_rpc::client;
use leptoskit_rpc::server::Identify;
use leptoskit_rpc::server::Identity;
use leptoskit_rpc::server::Server;

/// Who the request is from.
#[derive(Clone)]
struct Me(String);

/// Says hello to whoever is calling.
#[rpc::query]
async fn hello(cx: &rpc::QueryCx, punctuation: String) -> rpc::Result<String> {
	let me = cx.require::<Me>()?;
	Ok(format!("hello, {}{punctuation}", me.0))
}

/// Echoes, once per key.
#[rpc::mutation]
async fn shout(cx: &rpc::MutationCx, word: String) -> rpc::Result<String> {
	Ok(format!("{}{}", word.to_uppercase(), cx.attempt()))
}

struct Header;

impl Identify for Header {
	fn identify<'a>(
		&'a self,
		request: &'a http::request::Parts,
	) -> BoxFuture<'a, rpc::Result<Identity>> {
		let user = request
			.headers
			.get("x-user")
			.and_then(|value| value.to_str().ok())
			.map(str::to_owned);
		Box::pin(async move {
			Ok(match user {
				Some(user) => {
					Identity::new(Context::new().with(Me(user.clone()))).with_caller(user)
				}
				None => Identity::anonymous(),
			})
		})
	}
}

#[tokio::test]
async fn rendering_calls_run_in_process_as_the_request() {
	// Before any server is set up, calls say what's missing.
	let error = client::call(Hello {
		punctuation: "!".to_owned(),
	})
	.await
	.unwrap_err();
	assert!(error.message().contains("no server is set up"), "{error}");

	let db = leptoskit_db::testing::sqlite().await;
	let server = Server::new(db).identify(Header);
	server.install();
	assert!(format!("{server:?}").contains("Server"));

	// Outside a request: anonymous.
	assert_eq!(
		client::call(Hello {
			punctuation: "!".to_owned()
		})
		.await,
		Err(FnError::Server("sign in to do this".to_owned()))
	);
	assert_eq!(
		client::call(Shout {
			word: "hey".to_owned()
		})
		.await,
		Ok("HEY0".to_owned())
	);

	// Inside one, as leptos provides it while rendering.
	let owner = leptos::prelude::Owner::new();
	let (parts, ()) = http::Request::builder()
		.header("x-user", "ada")
		.body(())
		.unwrap()
		.into_parts();
	let call = owner.with(|| {
		leptos::prelude::provide_context(parts);
		client::call(Hello {
			punctuation: "?".to_owned(),
		})
	});
	assert_eq!(call.await, Ok("hello, ada?".to_owned()));

	let keyed = client::call_with(
		Shout {
			word: "once".to_owned(),
		},
		client::CallOptions::new().idempotency_key("k"),
	)
	.await;
	assert_eq!(keyed, Ok("ONCE0".to_owned()));
}
