//! The streams a server knows, and how one runs: its items go out only as
//! fast as the client grants credits, so the server never reads ahead of
//! what the client has room for.

use std::sync::Arc;

use futures::Stream;
use futures::StreamExt;
use leptoskit_query::BoxFuture;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio::sync::mpsc;

use super::QueryCx;
use crate::Error;
use crate::Result;
use crate::StreamOperation;
use crate::live::EndReason;
use crate::live::ServerFrame;
use crate::live::WireError;
use crate::stream::Resumable;

/// One `#[rpc::stream]`, as a server build registers it.
pub struct StreamRegistration {
	/// The stream's name.
	pub name: &'static str,
	/// Whether a reconnecting client opens it again from its cursor.
	pub resumable: bool,
	/// Decodes the arguments, runs the body as the connection's identity,
	/// and sends its items through the sink until it ends.
	#[doc(hidden)]
	pub open: Open,
}

/// What a registration runs.
pub type Open = fn(Value, QueryCx, StreamSink) -> BoxFuture<'static, ()>;

inventory::collect!(StreamRegistration);

/// The stream called `name`.
#[must_use]
pub fn registration(name: &str) -> Option<&'static StreamRegistration> {
	inventory::iter::<StreamRegistration>().find(|registration| registration.name == name)
}

/// Decodes the arguments of `S` and pumps it: the `open` a
/// `#[rpc::stream]` registers.
#[doc(hidden)]
pub fn open<S: StreamOperation>(
	args: Value,
	cx: QueryCx,
	sink: StreamSink,
) -> BoxFuture<'static, ()> {
	Box::pin(async move {
		match serde_json::from_value::<S>(args) {
			Ok(op) => op.pump(cx, sink).await,
			Err(error) => {
				sink.fail(&Error::invalid(format!(
					"the arguments of `{}` don't fit: {error}",
					S::NAME
				)));
			}
		}
	})
}

/// Where a running stream sends its items: the connection's session, gated
/// by the credits its client granted.
pub struct StreamSink {
	stream: u32,
	/// Unbounded, because the credits bound it: at most one item per credit
	/// is ever in it.
	out: mpsc::UnboundedSender<String>,
	credits: Arc<Semaphore>,
}

/// An item, encoded: its value, and where a resumable stream reached.
#[doc(hidden)]
pub type Encoded = Result<(Value, Option<Value>)>;

impl StreamSink {
	#[cfg_attr(
		not(feature = "live"),
		allow(dead_code, reason = "only a live session opens streams")
	)]
	pub(crate) fn new(
		stream: u32,
		out: mpsc::UnboundedSender<String>,
		credits: Arc<Semaphore>,
	) -> Self {
		Self {
			stream,
			out,
			credits,
		}
	}

	/// Sends every item of `items`, each once the client has room for it,
	/// and then the stream's end: done when `items` runs out, failed at the
	/// first item `encode` refuses.
	#[doc(hidden)]
	pub async fn drive<S, I>(self, items: S, encode: fn(I) -> Encoded)
	where
		S: Stream<Item = I> + Send,
	{
		let mut items = std::pin::pin!(items);

		loop {
			// Wait for room before reading the next item, so nothing is read
			// ahead of what the client asked for.
			match self.credits.acquire().await {
				Ok(permit) => permit.forget(),
				Err(_) => return,
			}

			let Some(item) = items.next().await else {
				self.end(EndReason::Done);
				return;
			};

			match encode(item) {
				Ok((value, cursor)) => {
					let frame = ServerFrame::Item {
						stream: self.stream,
						value,
						cursor,
					};

					if !self.send(&frame) {
						return;
					}
				}
				Err(error) => {
					self.fail(&error);
					return;
				}
			}
		}
	}

	/// Ends the stream as failed, as when its body refused to start.
	#[doc(hidden)]
	pub fn fail(&self, error: &Error) {
		self.end(EndReason::Failed(WireError {
			status: error.status(),
			message: error.message().to_owned(),
		}));
	}

	fn end(&self, reason: EndReason) {
		self.send(&reason.frame(self.stream));
	}

	fn send(&self, frame: &ServerFrame) -> bool {
		let Ok(text) = serde_json::to_string(frame) else {
			return false;
		};
		self.out.send(text).is_ok()
	}
}

fn encode<T: Serialize>(item: &T) -> Result<Value> {
	serde_json::to_value(item).map_err(|_| Error::server("a stream's item isn't JSON"))
}

/// An item that can't fail.
#[doc(hidden)]
pub fn plain<T: Serialize>(item: T) -> Encoded {
	Ok((encode(&item)?, None))
}

/// An item that may be the error the stream fails with.
#[doc(hidden)]
pub fn fallible<T: Serialize>(item: Result<T>) -> Encoded {
	plain(item?)
}

/// A resumable stream's item, with its cursor.
#[doc(hidden)]
#[expect(
	clippy::needless_pass_by_value,
	reason = "every encoder takes the item the stream yields, by value"
)]
pub fn resumable<T: Serialize + Resumable>(item: T) -> Encoded {
	let cursor = serde_json::to_value(item.cursor())
		.map_err(|_| Error::server("a stream's cursor isn't JSON"))?;
	Ok((encode(&item)?, Some(cursor)))
}

/// A resumable stream's item that may be the error the stream fails with.
#[doc(hidden)]
pub fn resumable_fallible<T: Serialize + Resumable>(item: Result<T>) -> Encoded {
	resumable(item?)
}
