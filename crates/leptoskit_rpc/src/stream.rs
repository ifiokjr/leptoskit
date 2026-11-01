//! Streams: server output over time that isn't a query over rows, such as a
//! job's progress, a model's tokens, or a log tail.
//!
//! ```ignore
//! /// A model's answer, a token at a time.
//! #[rpc::stream]
//! pub async fn answer(cx: &rpc::QueryCx, prompt: String) -> rpc::Result<impl Stream<Item = String>> {
//! 	Ok(my_llm::tokens(prompt))
//! }
//!
//! // Any client:
//! let answer = rpc::use_stream(move || Answer { prompt: prompt.get() });
//! let text = move || answer.items().get().concat();
//! ```
//!
//! A stream travels on the live connection, beside the app's queries, and
//! the server reads the next item only once the client has room for it, so
//! a slow phone never makes the server hold more than it asked for. It ends
//! as [`EndReason`](crate::live::EndReason) says: done, failed, or
//! interrupted by a reconnect. `#[rpc::stream(resumable)]` takes a `cursor`
//! argument, and its items say where they reached through [`Resumable`], so
//! the client opens it again from the last item it received.
//!
//! When the state could be a row, such as an import's progress, prefer a
//! live query: it survives reconnects and server restarts for free.

use serde::Serialize;
use serde::de::DeserializeOwned;

/// A stream's arguments, as one value: the struct `#[rpc::stream]`
/// generates from the function's arguments.
///
/// The struct exists on every target, so any client can open it with
/// [`use_stream`](crate::client::use_stream). Only a server build knows
/// how to run it.
pub trait StreamOperation: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {
	/// The stream's name, which the client opens it by.
	const NAME: &'static str;
	/// Whether a reconnecting client opens it again from its last item's
	/// cursor: `#[rpc::stream(resumable)]`.
	const RESUMABLE: bool = false;
	/// What each item is.
	type Item: Serialize + DeserializeOwned + Send + 'static;

	/// Runs the body as the caller, and sends its items through `sink`
	/// until it ends.
	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	#[doc(hidden)]
	fn pump(
		self,
		cx: crate::QueryCx,
		sink: crate::server::StreamSink,
	) -> impl Future<Output = ()> + Send;
}

/// An item of a resumable stream: where the stream reached with it.
///
/// A reconnecting client opens the stream again with its `cursor` argument
/// set to the last item's cursor, so the body carries on from there.
///
/// ```ignore
/// #[derive(Clone, Serialize, Deserialize)]
/// pub struct LogLine { pub offset: u64, pub text: String }
///
/// impl rpc::Resumable for LogLine {
/// 	type Cursor = u64;
/// 	fn cursor(&self) -> u64 { self.offset }
/// }
///
/// #[rpc::stream(resumable)]
/// pub async fn tail(cx: &rpc::QueryCx, file: String, cursor: Option<u64>) -> rpc::Result<impl Stream<Item = LogLine>> { … }
/// ```
pub trait Resumable {
	/// What the stream's `cursor` argument takes.
	type Cursor: Serialize + DeserializeOwned;

	/// Where the stream reached with this item.
	fn cursor(&self) -> Self::Cursor;
}

/// Checks a stream's item type, where the macro names it.
#[doc(hidden)]
pub const fn assert_stream_item<T: Serialize + DeserializeOwned + Send + 'static>() {}

/// Checks a resumable stream's `cursor` argument takes its items' cursor.
#[doc(hidden)]
pub const fn assert_cursor<I: Resumable, A: From<Option<I::Cursor>>>() {}
