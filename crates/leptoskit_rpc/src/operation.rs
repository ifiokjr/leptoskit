//! Operations: what an app exposes, by name and kind, with typed arguments
//! and a typed result.

use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::FnError;

/// What kind of operation something is, which decides how it runs and how
/// clients may call it.
///
/// | Kind                    | Declared with      | Runs                                                                 |
/// | ----------------------- | ------------------ | -------------------------------------------------------------------- |
/// | [`Call`](Self::Call)    | `#[server]`        | as a leptos server function, with its own encoding                   |
/// | [`Query`](Self::Query)  | `#[rpc::query]`    | reading the database as the caller, read-only by type                |
/// | [`Mutation`](Self::Mutation) | `#[rpc::mutation]` | in one serializable transaction, retried when another gets in its way |
/// | [`Action`](Self::Action) | `#[rpc::action]` | as the caller, outside any transaction, once: it may call anything |
/// | [`Job`](Self::Job)         | `#[rpc::job]`      | later, in the background, when something enqueues it |
///
/// More kinds will join these, such as streams, so matching on a `Kind`
/// needs a wildcard arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Kind {
	/// A leptos `#[server]` function.
	Call,
	/// An `#[rpc::query]`.
	Query,
	/// An `#[rpc::mutation]`.
	Mutation,
	/// An `#[rpc::action]`: not transactional and never retried, so it may
	/// call the outside world; it reaches the database through queries and
	/// mutations.
	Action,
	/// An `#[rpc::job]`: work that runs later, in the background, not a
	/// route a client calls.
	Job,
}

impl Kind {
	/// The kind's name in the spec: `call`, `query`, `mutation`, `action`,
	/// or `job`.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Call => "call",
			Self::Query => "query",
			Self::Mutation => "mutation",
			Self::Action => "action",
			Self::Job => "job",
		}
	}
}

impl fmt::Display for Kind {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.as_str())
	}
}

/// An operation's arguments, as one value: the struct `#[rpc::query]` and
/// `#[rpc::mutation]` generate from the function's arguments, named in
/// `PascalCase`.
///
/// The struct exists on every target, so any client can build one and send
/// it with [`client::call`](crate::client::call). Only a server build (the
/// `server` feature) knows how to [run](Self::run) it.
///
/// ```ignore
/// #[rpc::query]
/// pub async fn messages(cx: &rpc::QueryCx, channel: Id<Channel>, limit: u32) -> rpc::Result<Vec<PublicMessage>> { … }
///
/// // Anywhere: the arguments, by name.
/// let latest = rpc::client::call(Messages { channel, limit: 50 }).await?;
/// ```
pub trait Operation: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {
	/// The operation's name: its route is `/api/<NAME>`.
	const NAME: &'static str;
	/// What kind of operation it is.
	const KIND: Kind;
	/// What a successful call returns.
	type Output: Serialize + DeserializeOwned + Send + Sync + 'static;

	/// What the body receives: [`QueryCx`](crate::QueryCx) or
	/// [`MutationCx`](crate::MutationCx).
	#[cfg(feature = "server")]
	type Context: crate::server::OperationContext;

	/// How many more times a mutation runs when another transaction gets in
	/// its way: `#[rpc::mutation(retries = N)]`, five by default. Queries
	/// never retry.
	#[cfg(feature = "server")]
	const RETRIES: u32 = 5;

	/// Runs the body. Prefer [`call`](crate::call), which sets up the
	/// context: a mutation's transaction, its retries, and its receipt.
	#[cfg(feature = "server")]
	fn run(self, cx: &Self::Context) -> impl Future<Output = Result<Self::Output>> + Send;
}

/// What an operation's body returns.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Why an operation failed, as its body reports it.
///
/// Every variant reaches the caller with an HTTP status and the message,
/// classified as [`FnError`] classifies a server function's failure:
/// [`Invalid`](Self::Invalid) as [`FnError::Invalid`], the rest as
/// [`FnError::Server`]. So generated clients and the app's own client handle
/// them as they handle any server function's.
///
/// A database error converts with `?`: a row that isn't there becomes
/// [`NotFound`](Self::NotFound), a policy's refusal
/// [`Forbidden`](Self::Forbidden), a value that breaks a rule
/// [`Invalid`](Self::Invalid), a taken unique value or a changed row
/// [`Conflict`](Self::Conflict), and anything else a
/// [`Server`](Self::Server) error that says nothing of the database.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
	/// The arguments were wrong: `400`.
	Invalid(String),
	/// The caller must sign in first: `401`.
	Unauthorized(String),
	/// The caller may not do this: `403`.
	Forbidden(String),
	/// What the caller asked about doesn't exist, or they can't see it: `404`.
	NotFound(String),
	/// The request clashes with what is there: a taken name, a changed row:
	/// `409`.
	Conflict(String),
	/// Another transaction kept getting in the mutation's way, so it gave up
	/// after its retries: `409`. Trying again later can succeed.
	Contended,
	/// The server failed: `500`.
	Server(String),
	/// Something outside the server failed for now, such as a network call:
	/// `503`. A job that fails with it runs again after its backoff, as it
	/// does for any error but [`Permanent`](Self::Permanent).
	Retry(String),
	/// Trying again won't help, such as a payment the provider declined:
	/// `500`. A job that fails with it stops at once, without its remaining
	/// retries, and its `on_failure` mutation runs.
	Permanent(String),
}

impl Error {
	/// The arguments were wrong.
	pub fn invalid(message: impl Into<String>) -> Self {
		Self::Invalid(message.into())
	}

	/// The caller must sign in first.
	pub fn unauthorized(message: impl Into<String>) -> Self {
		Self::Unauthorized(message.into())
	}

	/// The caller may not do this.
	pub fn forbidden(message: impl Into<String>) -> Self {
		Self::Forbidden(message.into())
	}

	/// What the caller asked about doesn't exist.
	pub fn not_found(message: impl Into<String>) -> Self {
		Self::NotFound(message.into())
	}

	/// The request clashes with what is there.
	pub fn conflict(message: impl Into<String>) -> Self {
		Self::Conflict(message.into())
	}

	/// The server failed.
	pub fn server(message: impl Into<String>) -> Self {
		Self::Server(message.into())
	}

	/// Something outside failed for now: a job runs again after its backoff.
	pub fn retry(message: impl Into<String>) -> Self {
		Self::Retry(message.into())
	}

	/// Trying again won't help: a job stops without its remaining retries.
	pub fn permanent(message: impl Into<String>) -> Self {
		Self::Permanent(message.into())
	}

	/// Whether a job that failed with this should stop at once.
	#[must_use]
	pub fn is_permanent(&self) -> bool {
		matches!(self, Self::Permanent(_))
	}

	/// The HTTP status a caller receives.
	#[must_use]
	pub fn status(&self) -> u16 {
		match self {
			Self::Invalid(_) => 400,
			Self::Unauthorized(_) => 401,
			Self::Forbidden(_) => 403,
			Self::NotFound(_) => 404,
			Self::Conflict(_) | Self::Contended => 409,
			Self::Server(_) | Self::Permanent(_) => 500,
			Self::Retry(_) => 503,
		}
	}

	/// The message a caller reads.
	#[must_use]
	pub fn message(&self) -> &str {
		match self {
			Self::Invalid(message)
			| Self::Unauthorized(message)
			| Self::Forbidden(message)
			| Self::NotFound(message)
			| Self::Conflict(message)
			| Self::Server(message)
			| Self::Retry(message)
			| Self::Permanent(message) => message,
			Self::Contended => "the server was too busy to finish this; try again",
		}
	}

	/// Whether another transaction got in the way, so running the mutation
	/// again can succeed.
	#[must_use]
	pub fn is_contended(&self) -> bool {
		matches!(self, Self::Contended)
	}

	/// The body a caller receives: `Kind|message`, where `Kind` is the
	/// `ServerFnError` variant generated clients classify, as
	/// [`FnError::from_wire`] reads it.
	#[must_use]
	pub fn to_wire(&self) -> String {
		let kind = match self {
			Self::Invalid(_) => "Args",
			_ => "ServerError",
		};
		format!("{kind}|{}", self.message())
	}
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.message())
	}
}

impl std::error::Error for Error {}

impl From<Error> for FnError {
	fn from(error: Error) -> Self {
		Self::from_wire(&error.to_wire())
	}
}

#[cfg(feature = "db")]
impl From<leptoskit_db::DbError> for Error {
	fn from(error: leptoskit_db::DbError) -> Self {
		use leptoskit_db::DbError;

		if error.is_contended() {
			return Self::Contended;
		}

		match error {
			DbError::NotFound { .. } => Self::NotFound(error.to_string()),
			DbError::Forbidden { message, .. } => Self::Forbidden(message),
			DbError::Invalid(_) | DbError::Check { .. } | DbError::ForeignKey { .. } => {
				Self::Invalid(error.to_string())
			}
			DbError::Conflict { .. } | DbError::Stale { .. } => Self::Conflict(error.to_string()),
			// The rest are the server's own failures, and the caller hears
			// only that something went wrong.
			_ => Self::Server("something went wrong; try again".to_owned()),
		}
	}
}

/// Arguments an operation can take: they cross the wire as JSON, and the
/// spec describes them.
#[diagnostic::on_unimplemented(
	message = "`{Self}` can't be an operation's argument",
	label = "arguments must implement `Serialize`, `Deserialize`, and `JsonSchema`",
	note = "an operation's arguments travel as JSON and are described in the RPC spec; derive \
	        `serde::Serialize`, `serde::Deserialize`, and `schemars::JsonSchema` for it"
)]
#[doc(hidden)]
pub trait Argument: Serialize + DeserializeOwned + schemars::JsonSchema {}

impl<T: Serialize + DeserializeOwned + schemars::JsonSchema> Argument for T {}

/// Results an operation can return: they cross the wire as JSON.
#[diagnostic::on_unimplemented(
	message = "`{Self}` can't be an operation's result",
	label = "results must implement `Serialize` and `Deserialize`",
	note = "an operation's result travels back as JSON; derive `serde::Serialize` and \
	        `serde::Deserialize` for it"
)]
#[doc(hidden)]
pub trait Output: Serialize + DeserializeOwned + Send + Sync + 'static {}

impl<T: Serialize + DeserializeOwned + Send + Sync + 'static> Output for T {}

/// Checks an argument type, where the macro names it.
#[doc(hidden)]
pub const fn assert_argument<T: Argument>() {}

/// Checks a result type, where the macro names it.
#[doc(hidden)]
pub const fn assert_output<T: Output>() {}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn errors_reach_callers_classified_like_server_functions() {
		let cases = [
			(Error::invalid("too long"), 400, "Args|too long"),
			(Error::unauthorized("sign in"), 401, "ServerError|sign in"),
			(Error::forbidden("not yours"), 403, "ServerError|not yours"),
			(
				Error::not_found("no such post"),
				404,
				"ServerError|no such post",
			),
			(Error::conflict("taken"), 409, "ServerError|taken"),
			(Error::server("down"), 500, "ServerError|down"),
		];
		for (error, status, wire) in cases {
			assert_eq!(error.status(), status);
			assert_eq!(error.to_wire(), wire);
			assert_eq!(error.to_string(), error.message());
			assert!(!error.is_contended());
		}
		assert_eq!(Error::Contended.status(), 409);
		assert!(Error::Contended.is_contended());
		assert!(Error::Contended.to_wire().starts_with("ServerError|"));
		assert_eq!(
			FnError::from(Error::invalid("bad")),
			FnError::Invalid("bad".to_owned())
		);
		assert_eq!(
			FnError::from(Error::not_found("gone")),
			FnError::Server("gone".to_owned())
		);
	}

	#[test]
	fn kinds_have_their_spec_names() {
		assert_eq!(Kind::Call.to_string(), "call");
		assert_eq!(Kind::Query.as_str(), "query");
		assert_eq!(Kind::Mutation.as_str(), "mutation");
		assert_eq!(Kind::Action.as_str(), "action");
		assert_eq!(Kind::Job.as_str(), "job");
	}

	#[test]
	fn jobs_tell_retrying_from_giving_up() {
		let retry = Error::retry("the mail server is down");
		assert_eq!(retry.status(), 503);
		assert_eq!(retry.message(), "the mail server is down");
		assert!(!retry.is_permanent());
		let permanent = Error::permanent("declined");
		assert_eq!(permanent.status(), 500);
		assert_eq!(permanent.to_wire(), "ServerError|declined");
		assert!(permanent.is_permanent());
		assert!(!Error::server("down").is_permanent());
	}

	#[cfg(feature = "db")]
	#[test]
	fn database_errors_say_nothing_of_the_database() {
		use leptoskit_db::DbError;

		assert!(matches!(
			Error::from(DbError::NotFound {
				table: "posts".to_owned()
			}),
			Error::NotFound(_)
		));
		assert_eq!(
			Error::from(DbError::Forbidden {
				table: "posts".to_owned(),
				message: "Only the author can change a post".to_owned(),
			}),
			Error::forbidden("Only the author can change a post")
		);
		assert!(matches!(
			Error::from(DbError::Stale {
				table: "posts".to_owned()
			}),
			Error::Conflict(_)
		));
		assert_eq!(
			Error::from(DbError::Query("relation \"posts\" has a secret".into())),
			Error::server("something went wrong; try again")
		);
	}
}
