//! Who is calling: a request becomes the [`Context`] policies read, and an
//! ID that a mutation's receipts belong to.

use std::sync::Arc;

use leptoskit_db::Context;
use leptoskit_query::BoxFuture;

use crate::Error;

/// Who an operation runs as.
#[derive(Clone, Default)]
pub struct Identity {
	/// What policies read: the signed-in person, their tenant, and anything
	/// else the app adds.
	pub context: Context,
	/// Who the call's receipts belong to: a session ID when there is one.
	/// Two calls with the same idempotency key are the same call only if
	/// they come from the same caller. `None` for anonymous callers, who
	/// share one space of keys.
	pub caller: Option<String>,
}

impl Identity {
	/// Nobody: policies that read a global match no rows.
	pub fn anonymous() -> Self {
		Self::default()
	}

	/// Someone, as policies see them.
	pub fn new(context: Context) -> Self {
		Self {
			context,
			caller: None,
		}
	}

	/// The identity, with the ID its receipts belong to.
	#[must_use]
	pub fn with_caller(mut self, caller: impl Into<String>) -> Self {
		self.caller = Some(caller.into());
		self
	}

	/// The identity a database handle acts as: its context, or nobody's
	/// for a handle that bypasses policies.
	pub fn of(db: &impl HasContext) -> Self {
		Self::new(db.context_or_default())
	}
}

impl std::fmt::Debug for Identity {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Identity")
			.field("caller", &self.caller)
			.finish_non_exhaustive()
	}
}

/// A database handle that may act for someone.
#[doc(hidden)]
pub trait HasContext {
	fn context_or_default(&self) -> Context;
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
impl HasContext for leptoskit_db::Db {
	fn context_or_default(&self) -> Context {
		self.context().cloned().unwrap_or_default()
	}
}

/// Resolves a request to who it is from.
///
/// The RPC calls it for every operation it serves, and the database handle
/// the operation runs on obeys the policies of the [`Identity`] it returns.
/// `leptoskit_auth` implements it, with its `rpc` feature, for its `Auth`:
/// a session cookie or a bearer token becomes the signed-in person.
///
/// ```ignore
/// struct ApiKeys(Db);
///
/// impl rpc::Identify for ApiKeys {
/// 	fn identify<'a>(&'a self, request: &'a http::request::Parts) -> BoxFuture<'a, rpc::Result<rpc::Identity>> {
/// 		Box::pin(async move {
/// 			let Some(key) = request.headers.get("x-api-key") else {
/// 				return Ok(rpc::Identity::anonymous());
/// 			};
/// 			let tenant = lookup(&self.0, key).await?.ok_or_else(|| rpc::Error::unauthorized("unknown key"))?;
/// 			Ok(rpc::Identity::new(Context::new().with(tenant)).with_caller(key.to_str().unwrap_or_default()))
/// 		})
/// 	}
/// }
/// ```
pub trait Identify: Send + Sync + 'static {
	/// Who an HTTP request is from. Read headers and the extensions an
	/// earlier layer added; the body is the operation's arguments.
	///
	/// # Errors
	///
	/// [`Error::Unauthorized`] for credentials that don't hold, which ends
	/// the call. A request with no credentials is usually
	/// [`Identity::anonymous`] instead, and policies decide what it may see.
	fn identify<'a>(
		&'a self,
		request: &'a http::request::Parts,
	) -> BoxFuture<'a, Result<Identity, Error>>;

	/// Who a bearer token belongs to: how a connection that isn't one HTTP
	/// request, such as a socket, says who it is. By default it asks
	/// [`identify`](Self::identify) about a request carrying `Authorization:
	/// Bearer <token>`.
	///
	/// # Errors
	///
	/// As [`identify`](Self::identify).
	fn identify_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<Identity, Error>> {
		Box::pin(async move {
			let mut request = http::Request::new(());
			let value = http::HeaderValue::from_str(&format!("Bearer {token}"))
				.map_err(|_| Error::unauthorized("the token isn't valid in a header"))?;
			request
				.headers_mut()
				.insert(http::header::AUTHORIZATION, value);
			let (parts, ()) = request.into_parts();
			self.identify(&parts).await
		})
	}
}

impl<I: Identify> Identify for Arc<I> {
	fn identify<'a>(
		&'a self,
		request: &'a http::request::Parts,
	) -> BoxFuture<'a, Result<Identity, Error>> {
		(**self).identify(request)
	}

	fn identify_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<Identity, Error>> {
		(**self).identify_token(token)
	}
}

/// Everyone is anonymous: the default until the server says how to tell
/// callers apart.
#[derive(Clone, Copy, Debug, Default)]
pub struct Anonymous;

impl Identify for Anonymous {
	fn identify<'a>(
		&'a self,
		_request: &'a http::request::Parts,
	) -> BoxFuture<'a, Result<Identity, Error>> {
		Box::pin(std::future::ready(Ok(Identity::anonymous())))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Trusts an `x-user` header, for the test.
	struct Header;

	impl Identify for Header {
		fn identify<'a>(
			&'a self,
			request: &'a http::request::Parts,
		) -> BoxFuture<'a, Result<Identity, Error>> {
			Box::pin(async move {
				let header = request
					.headers
					.get(http::header::AUTHORIZATION)
					.and_then(|value| value.to_str().ok())
					.ok_or_else(|| Error::unauthorized("no token"))?;
				Ok(Identity::new(Context::new().with(header.to_owned())).with_caller(header))
			})
		}
	}

	#[tokio::test]
	async fn a_token_identifies_as_a_bearer_request_would() {
		let identity = Arc::new(Header)
			.identify_token("abc")
			.await
			.expect("a token");
		assert_eq!(identity.caller.as_deref(), Some("Bearer abc"));
		assert_eq!(
			identity.context.get::<String>().map(String::as_str),
			Some("Bearer abc")
		);
		assert!(format!("{identity:?}").contains("Bearer abc"));
		assert!(matches!(
			Header.identify_token("bad\ntoken").await,
			Err(Error::Unauthorized(_))
		));

		let (parts, ()) = http::Request::new(()).into_parts();
		let nobody = Anonymous.identify(&parts).await.expect("anonymous");
		assert!(nobody.caller.is_none());
		assert!(Identity::anonymous().context.get::<String>().is_none());
	}
}
