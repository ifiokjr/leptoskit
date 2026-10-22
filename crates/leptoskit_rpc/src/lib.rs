//! An ergonomic API over leptos server functions.
//!
//! A leptos server function is already a typed, isomorphic RPC: `#[server]`
//! generates a struct whose `FromArg`/`await` call handles serialization,
//! transport, and routing on both sides. What it leaves to you is everything
//! around the call — retrying flaky networks, classifying which failures are
//! worth retrying, and wrapping results so loading and failure states are one
//! type. This crate adds exactly that layer:
//!
//! - [`query`] turns any server-fn call into a reactive [`Query`] with a retry
//!   policy, ready for `<Await>`-style rendering;
//! - [`mutation`] and [`mutation_with_retry`] wrap calls in a leptos [`Action`]
//!   with the same error handling;
//! - [`FnError`] classifies failures so "offline" and "your input was invalid"
//!   stop looking identical in the UI;
//! - [`api`] publishes a server function to generated clients — the same RPC,
//!   described, with docs — so `leptoskit rpc generate` can write typed clients
//!   for TypeScript, Python, Go, Rust, Dart, and the command line (see the
//!   [`spec`] module, behind the `ssr` feature).
//!
//! ```no_run
//! use leptos::prelude::*;
//! use leptoskit_rpc::FnError;
//! use leptoskit_rpc::RetryPolicy;
//! use leptoskit_rpc::mutation;
//! use leptoskit_rpc::query;
//!
//! # async fn list_todos() -> Result<Vec<String>, ServerFnError> { unimplemented!() }
//! # async fn create_todo(title: String) -> Result<(), ServerFnError> { unimplemented!() }
//! # use leptos::prelude::ServerFnError;
//! // Reads retry automatically and stay reactive.
//! let todos = query(RetryPolicy::aggressive(), || {
//! 	async { list_todos().await.map_err(FnError::from) }
//! });
//!
//! // Mutations only retry when asked, because re-sending a POST blindly is
//! // usually wrong.
//! let add = mutation(|title: &String| {
//! 	let title = title.clone();
//! 	async move { create_todo(title).await.map_err(FnError::from) }
//! });
//! ```
//!
//! # Publishing the RPC
//!
//! Put `#[api]` above `#[server]` and the function becomes part of the app's
//! published RPC: a stable `/api/<name>` route, a JSON Schema description of
//! its arguments and result, and its doc comment — including the
//! `# Arguments` section — as documentation. `#[api(internal)]` keeps a
//! function callable but takes it out of generated clients. Enable the `ssr`
//! feature next to `leptos/ssr`, call
//! `leptoskit_rpc::spec::export_if_requested()` first in `main`, and
//! `leptoskit rpc generate` writes typed clients for any language that speak
//! the RPC's own protocol. See the book's "Sharing the RPC" chapter.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use leptos::prelude::Action;
use leptos::prelude::ArcAsyncDerived;
use leptos::prelude::ServerFnError;
pub use leptoskit_rpc_macros::api;

#[cfg(feature = "ssr")]
pub mod spec;

/// Re-exports the `#[api]` macro and the generated clients expand to; not
/// part of the public API.
#[doc(hidden)]
pub mod __private {
	#[cfg(feature = "ssr")]
	pub use inventory;
	pub use leptos::server_fn::ServerFn;
}

/// One classified server-fn failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FnError {
	/// The request never reached the server, or the connection dropped.
	Network(String),
	/// The server rejected the request; retrying will not help.
	Invalid(String),
	/// The server failed while running the function.
	Server(String),
	/// The response could not be decoded.
	Decode(String),
	/// Everything else.
	Other(String),
}

impl FnError {
	/// Whether retrying the call could plausibly succeed.
	#[must_use]
	pub fn retryable(&self) -> bool {
		matches!(self, Self::Network(_) | Self::Server(_))
	}

	/// The human-readable message.
	#[must_use]
	pub fn message(&self) -> &str {
		match self {
			Self::Network(message)
			| Self::Invalid(message)
			| Self::Server(message)
			| Self::Decode(message)
			| Self::Other(message) => message,
		}
	}

	/// Parses the server's error text, `Kind|message`, where `Kind` names a
	/// `ServerFnError` variant — the encoding every generated client sees.
	///
	/// ```
	/// use leptoskit_rpc::FnError;
	///
	/// assert_eq!(
	/// 	FnError::from_wire("Args|missing field `title`"),
	/// 	FnError::Invalid("missing field `title`".to_owned())
	/// );
	/// assert!(FnError::from_wire("ServerError|database down").retryable());
	/// assert_eq!(
	/// 	FnError::from_wire("plain text"),
	/// 	FnError::Other("plain text".to_owned())
	/// );
	/// ```
	#[must_use]
	pub fn from_wire(text: &str) -> Self {
		let Some((kind, message)) = text.split_once('|') else {
			return Self::Other(text.to_owned());
		};
		let message = message.to_owned();
		match kind {
			"Request" => Self::Network(message),
			"Args" | "MissingArg" => Self::Invalid(message),
			"ServerError" | "MiddlewareError" | "WrappedServerFn" => Self::Server(message),
			"Deserialization" | "Serialization" => Self::Decode(message),
			"Registration" | "Response" => Self::Other(message),
			_ => Self::Other(text.to_owned()),
		}
	}
}

impl std::fmt::Display for FnError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let kind = match self {
			Self::Network(_) => "network",
			Self::Invalid(_) => "invalid",
			Self::Server(_) => "server",
			Self::Decode(_) => "decode",
			Self::Other(_) => "other",
		};
		write!(f, "{kind} error: {}", self.message())
	}
}

impl std::error::Error for FnError {}

impl From<ServerFnError> for FnError {
	fn from(error: ServerFnError) -> Self {
		#[allow(deprecated)]
		match error {
			ServerFnError::Request(message) => Self::Network(message),
			ServerFnError::Args(message) | ServerFnError::MissingArg(message) => {
				Self::Invalid(message)
			}
			ServerFnError::ServerError(message) | ServerFnError::MiddlewareError(message) => {
				Self::Server(message)
			}
			ServerFnError::Deserialization(message) | ServerFnError::Serialization(message) => {
				Self::Decode(message)
			}
			ServerFnError::Registration(message) | ServerFnError::Response(message) => {
				Self::Other(message)
			}
			ServerFnError::WrappedServerError(inner) => Self::Server(inner.to_string()),
		}
	}
}

/// How a retrying call backs off between attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
	/// The total number of attempts, including the first.
	pub attempts: u32,
	/// The base delay between attempts; it doubles every retry.
	pub base_delay_ms: u64,
	/// The largest delay between attempts.
	pub max_delay_ms: u64,
}

impl RetryPolicy {
	/// A single attempt.
	#[must_use]
	pub const fn none() -> Self {
		Self {
			attempts: 1,
			base_delay_ms: 0,
			max_delay_ms: 0,
		}
	}

	/// Three attempts with a short backoff: the friendly default for reads.
	#[must_use]
	pub const fn reads() -> Self {
		Self {
			attempts: 3,
			base_delay_ms: 150,
			max_delay_ms: 2_000,
		}
	}

	/// Five attempts with a wider backoff, for flaky mobile networks.
	#[must_use]
	pub const fn aggressive() -> Self {
		Self {
			attempts: 5,
			base_delay_ms: 250,
			max_delay_ms: 10_000,
		}
	}

	/// The delay before attempt `retry` (zero-based), in milliseconds.
	#[must_use]
	pub fn delay_ms(&self, retry: u32) -> u64 {
		if retry == 0 {
			return 0;
		}
		let shift = retry.saturating_sub(1).min(16);
		self.base_delay_ms
			.saturating_mul(1_u64 << shift)
			.min(self.max_delay_ms)
	}
}

impl Default for RetryPolicy {
	fn default() -> Self {
		Self::reads()
	}
}

/// The future returned by [`call_with_retry`].
pub type RetryFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, FnError>> + Send + 'a>>;

/// Calls `fetch`, retrying retryable failures under `policy`.
///
/// `fetch` returns a fresh future every attempt; the error type is
/// [`FnError`] so classification decides what retries.
pub fn call_with_retry<T, F>(policy: RetryPolicy, fetch: F) -> RetryFuture<'static, T>
where
	T: Send + Sync + 'static,
	F: Fn() -> Pin<Box<dyn Future<Output = Result<T, FnError>> + Send>> + Send + Sync + 'static,
{
	Box::pin(async move {
		let fetch = Arc::new(fetch);
		let mut attempt = 0_u32;
		loop {
			let outcome = (fetch)().await;
			match outcome {
				Ok(value) => return Ok(value),
				Err(error) => {
					attempt += 1;
					if attempt >= policy.attempts || !error.retryable() {
						return Err(error);
					}
					sleep(Duration::from_millis(policy.delay_ms(attempt))).await;
				}
			}
		}
	})
}

/// Waits out a retry backoff on native targets.
#[cfg(not(target_arch = "wasm32"))]
async fn sleep(delay: Duration) {
	tokio::time::sleep(delay).await;
}

/// Waits out a retry backoff in the browser.
///
/// `tokio::time` has no driver on `wasm32-unknown-unknown` and panics, so the
/// browser waits on `setTimeout` instead. The timer callback completes a
/// oneshot channel, which keeps the returned future `Send`. This needs a
/// browser window: in a Web Worker, or in wasm hosted outside a browser, the
/// timer cannot be scheduled and the wait never completes.
#[cfg(target_arch = "wasm32")]
async fn sleep(delay: Duration) {
	let (sender, receiver) = futures::channel::oneshot::channel::<()>();
	leptos::prelude::set_timeout(
		move || {
			let _ = sender.send(());
		},
		delay,
	);
	let _ = receiver.await;
}

/// A reactive server-fn query: retrying, error-carrying, ready to render.
pub type Query<T> = ArcAsyncDerived<Result<T, FnError>>;

/// Turns a server-fn call into a reactive query under a retry policy.
///
/// The query re-runs when signals read inside `fetch` change, like any other
/// async derived value; each run gets the full retry policy.
pub fn query<T, Fu>(policy: RetryPolicy, fetch: impl Fn() -> Fu + Send + Sync + 'static) -> Query<T>
where
	T: Send + Sync + 'static,
	Fu: Future<Output = Result<T, FnError>> + Send + 'static,
{
	let fetch = Arc::new(fetch);
	ArcAsyncDerived::new(move || {
		let fetch = Arc::clone(&fetch);
		call_with_retry(policy, move || {
			let fut = fetch();
			Box::pin(fut)
		})
	})
}

/// A reactive server-fn mutation.
pub type Mutation<I, O> = Action<I, Result<O, FnError>>;

/// Wraps a server-fn call in a leptos action with unified errors.
///
/// Mutations do not retry by default: re-sending a POST blindly is usually
/// wrong. Use [`mutation_with_retry`] when the call is idempotent.
pub fn mutation<I, O, Fu>(action_fn: impl Fn(&I) -> Fu + Send + Sync + 'static) -> Mutation<I, O>
where
	I: Send + Sync + 'static,
	O: Send + Sync + 'static,
	Fu: Future<Output = Result<O, FnError>> + Send + 'static,
{
	Action::new(action_fn)
}

/// Wraps an idempotent server-fn call in a retrying action.
pub fn mutation_with_retry<I, O, Fu>(
	policy: RetryPolicy,
	action_fn: impl Fn(&I) -> Fu + Send + Sync + 'static,
) -> Mutation<I, O>
where
	I: Send + Sync + Clone + 'static,
	O: Send + Sync + 'static,
	Fu: Future<Output = Result<O, FnError>> + Send + 'static,
{
	let action_fn = Arc::new(action_fn);
	Action::new(move |input: &I| {
		let input = input.clone();
		let action_fn = Arc::clone(&action_fn);
		call_with_retry(policy, move || Box::pin(action_fn(&input)))
	})
}
