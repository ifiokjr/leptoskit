//! Cooperative cancellation for in-flight fetches.
//!
//! Dropping a Rust future cancels it, but a fetch runs inside a spawned task
//! that several observers may be awaiting, so the task itself cannot simply be
//! dropped. Instead every fetch carries a [`CancelToken`]: cancelling it wakes
//! the fetch loop, which stops retrying and lets the attempt's future drop.
//! Query functions can also poll the token to abort expensive work early.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

use parking_lot::Mutex;

/// A handle that cancels one fetch and everything awaiting it.
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
	inner: Arc<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
	cancelled: AtomicBool,
	wakers: Mutex<Vec<Waker>>,
}

impl CancelToken {
	/// A fresh, uncancelled token.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Cancels the fetch; later calls are no-ops.
	pub fn cancel(&self) {
		if !self.inner.cancelled.swap(true, Ordering::SeqCst) {
			let wakers = std::mem::take(&mut *self.inner.wakers.lock());

			for waker in wakers {
				waker.wake();
			}
		}
	}

	/// Whether [`cancel`](Self::cancel) was called.
	#[must_use]
	pub fn is_cancelled(&self) -> bool {
		self.inner.cancelled.load(Ordering::SeqCst)
	}

	/// Resolves once the token is cancelled.
	#[must_use]
	pub fn cancelled(&self) -> Cancelled {
		Cancelled {
			token: self.clone(),
		}
	}
}

/// The future returned by [`CancelToken::cancelled`].
#[derive(Debug)]
pub struct Cancelled {
	token: CancelToken,
}

impl Future for Cancelled {
	type Output = ();

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
		if self.token.is_cancelled() {
			return Poll::Ready(());
		}
		{
			let mut wakers = self.token.inner.wakers.lock();

			if !wakers.iter().any(|waker| waker.will_wake(cx.waker())) {
				wakers.push(cx.waker().clone());
			}
		}

		// A cancellation that landed between the first check and registering
		// the waker would otherwise be missed.
		if self.token.is_cancelled() {
			Poll::Ready(())
		} else {
			Poll::Pending
		}
	}
}
