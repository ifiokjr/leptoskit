//! The client while the server renders a page: it runs the operation in
//! process, as the request being rendered, with no HTTP hop.

use leptoskit_query::BoxFuture;
use serde_json::Value;

use super::Api;
use super::Key;
use crate::FnError;

/// The installed [`Server`](crate::server::Server), in process.
pub(crate) struct InProcess;

impl Api for InProcess {
	fn call(
		&self,
		name: &'static str,
		arguments: Value,
		key: Key,
	) -> BoxFuture<'static, Result<Value, FnError>> {
		// The request leptos is rendering, read before the first await, while
		// its context is current.
		let parts = leptos::prelude::use_context::<http::request::Parts>();
		Box::pin(async move {
			let server = crate::server::installed().ok_or_else(|| {
				FnError::Other(
					"no server is set up for in-process calls: build the routes with \
					 `rpc::server::Server::router`, or call `install` on it"
						.to_owned(),
				)
			})?;
			let registration = crate::server::registration(name).ok_or_else(|| {
				FnError::Other(format!("no operation is named `{name}` in this server"))
			})?;
			let parts = match parts {
				Some(parts) => parts,
				// Outside a request, such as in a test: an empty request,
				// which identifies as anonymous unless `Identify` says
				// otherwise.
				None => http::Request::new(()).into_parts().0,
			};

			server
				.call(&parts, registration, arguments, key.resolve().await)
				.await
				.map_err(FnError::from)
		})
	}
}
