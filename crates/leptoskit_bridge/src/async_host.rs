//! Optional native async handlers. No executor is created by this module.

use std::future::Future;
use std::pin::Pin;

pub use tokio_util::sync::CancellationToken;

use crate::Bytes;
use crate::Error;
use crate::MethodId;
use crate::Reply;

/// An independently owned asynchronous operation.
pub type PluginFuture<'a> = Pin<Box<dyn Future<Output = Result<Reply, Error>> + Send + 'a>>;

/// An async native plugin shared by independently cancellable calls.
///
/// Handlers must stop and release call-owned resources before resolving after
/// cancellation. A shell handler must kill and reap children; merely dropping
/// a process handle is insufficient. Implementations must never block a poll.
pub trait AsyncPlugin: Send + Sync {
	/// Stable registration name.
	fn name(&self) -> &'static str;
	/// Supported versioned operations.
	fn methods(&self) -> &'static [MethodId];
	/// Maximum complete encoded response, reserved before starting work.
	fn reply_limit(&self, method: MethodId) -> Result<usize, Error>;
	/// Executes a call. Check cancellation before starting any external work.
	fn invoke(
		&self,
		method: MethodId,
		payload: Bytes,
		cancellation: CancellationToken,
	) -> PluginFuture<'_>;
	/// Releases all persistent resources when the app unloads. Running calls
	/// have already been cancelled. This future must finish after cleanup.
	fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}
