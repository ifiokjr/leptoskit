use std::fmt;

use crate::Bytes;

/// Optional ownership attached to a resource-bearing response.
///
/// Dropping an uncommitted guard must release the provisional resource.
/// Cleanup must not block the bridge thread. `commit` runs only after the
/// caller successfully consumes the response, never during size inspection.
pub trait ReplyGuard: Send {
	/// Transfers the resource to the caller. Subsequent drop must not revoke
	/// it.
	fn commit(&mut self);
}

/// Owned response bytes and optional provisional resource ownership.
pub struct Reply {
	pub(crate) bytes: Bytes,
	pub(crate) guard: Option<Box<dyn ReplyGuard>>,
}

impl Reply {
	/// Attaches a resource lease to a response without changing its wire bytes.
	pub fn guarded(bytes: Bytes, guard: impl ReplyGuard + 'static) -> Self {
		Self {
			bytes,
			guard: Some(Box::new(guard)),
		}
	}

	/// Borrows the encoded response.
	pub fn bytes(&self) -> &Bytes {
		&self.bytes
	}

	/// Adds ownership around an existing response, preserving its resource
	/// lease.
	#[must_use]
	pub fn with_guard(self, guard: impl ReplyGuard + 'static) -> Self {
		Self::guarded(
			self.bytes,
			CombinedGuard {
				inner: self.guard,
				outer: guard,
			},
		)
	}
}

impl From<Bytes> for Reply {
	fn from(bytes: Bytes) -> Self {
		Self { bytes, guard: None }
	}
}

impl fmt::Debug for Reply {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("Reply")
			.field("bytes", &self.bytes)
			.field("guarded", &self.guard.is_some())
			.finish()
	}
}

struct CombinedGuard<G> {
	inner: Option<Box<dyn ReplyGuard>>,
	outer: G,
}

impl<G: ReplyGuard> ReplyGuard for CombinedGuard<G> {
	fn commit(&mut self) {
		if let Some(inner) = &mut self.inner {
			inner.commit();
		}
		self.outer.commit();
	}
}
