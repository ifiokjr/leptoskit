//! Retry policies and backoff.
//!
//! A failed attempt consults the [`Retry`] policy with the number of failures
//! so far (zero on the first failure) and the error, then waits for the
//! [`RetryDelay`] before trying again. The defaults match TanStack Query:
//! three retries in the browser, none on the server, with exponential backoff
//! from one second capped at thirty.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

/// How many times a failed fetch is retried before the error is surfaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryPolicy {
	/// Surface the first error.
	Never,
	/// Retry up to this many times.
	Count(u32),
	/// Retry until the fetch succeeds or is cancelled.
	Forever,
}

impl RetryPolicy {
	/// Whether another attempt should follow `failure_count` failures.
	#[must_use]
	pub fn should_retry(self, failure_count: u32) -> bool {
		match self {
			Self::Never => false,
			Self::Count(limit) => failure_count < limit,
			Self::Forever => true,
		}
	}
}

type RetryDecideFn<E> = Arc<dyn Fn(u32, &E) -> bool + Send + Sync>;

/// The retry decision for an error type.
pub enum Retry<E> {
	/// A fixed policy that ignores the error.
	Policy(RetryPolicy),
	/// Decides per failure from the failure count and the error.
	Decide(RetryDecideFn<E>),
}

impl<E> Clone for Retry<E> {
	fn clone(&self) -> Self {
		match self {
			Self::Policy(policy) => Self::Policy(*policy),
			Self::Decide(decide) => Self::Decide(Arc::clone(decide)),
		}
	}
}

impl<E> Retry<E> {
	/// Never retry.
	#[must_use]
	pub const fn never() -> Self {
		Self::Policy(RetryPolicy::Never)
	}

	/// Retry up to `count` times.
	#[must_use]
	pub const fn count(count: u32) -> Self {
		Self::Policy(RetryPolicy::Count(count))
	}

	/// Retry until success.
	#[must_use]
	pub const fn forever() -> Self {
		Self::Policy(RetryPolicy::Forever)
	}

	/// Decide per failure; the closure receives the failures so far and the
	/// error that just occurred.
	pub fn decide(decide: impl Fn(u32, &E) -> bool + Send + Sync + 'static) -> Self {
		Self::Decide(Arc::new(decide))
	}

	/// Whether another attempt should follow `failure_count` failures.
	#[must_use]
	pub fn should_retry(&self, failure_count: u32, error: &E) -> bool {
		match self {
			Self::Policy(policy) => policy.should_retry(failure_count),
			Self::Decide(decide) => decide(failure_count, error),
		}
	}
}

impl<E> fmt::Debug for Retry<E> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Policy(policy) => f.debug_tuple("Policy").field(policy).finish(),
			Self::Decide(_) => f.write_str("Decide(..)"),
		}
	}
}

impl<E> From<RetryPolicy> for Retry<E> {
	fn from(policy: RetryPolicy) -> Self {
		Self::Policy(policy)
	}
}

impl<E> From<u32> for Retry<E> {
	fn from(count: u32) -> Self {
		Self::count(count)
	}
}

impl<E> From<bool> for Retry<E> {
	fn from(retry: bool) -> Self {
		if retry {
			Self::forever()
		} else {
			Self::never()
		}
	}
}

/// Exponential backoff with a cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backoff {
	/// The delay before the first retry; it doubles on every failure.
	pub initial: Duration,
	/// The longest delay between attempts.
	pub max: Duration,
}

impl Backoff {
	/// Doubles from `initial` up to `max`.
	#[must_use]
	pub const fn exponential(initial: Duration, max: Duration) -> Self {
		Self { initial, max }
	}

	/// The delay before the retry that follows `failure_count` failures.
	#[must_use]
	pub fn delay(&self, failure_count: u32) -> Duration {
		let factor = 1_u32.checked_shl(failure_count.min(31)).unwrap_or(u32::MAX);
		self.initial
			.checked_mul(factor)
			.unwrap_or(self.max)
			.min(self.max)
	}
}

impl Default for Backoff {
	fn default() -> Self {
		Self::exponential(Duration::from_secs(1), Duration::from_secs(30))
	}
}

type RetryDelayFn<E> = Arc<dyn Fn(u32, &E) -> Duration + Send + Sync>;

/// How long to wait before each retry.
pub enum RetryDelay<E> {
	/// The same delay every time.
	Fixed(Duration),
	/// Exponential backoff.
	Backoff(Backoff),
	/// Decide per failure from the failure count and the error.
	Decide(RetryDelayFn<E>),
}

impl<E> Clone for RetryDelay<E> {
	fn clone(&self) -> Self {
		match self {
			Self::Fixed(delay) => Self::Fixed(*delay),
			Self::Backoff(backoff) => Self::Backoff(*backoff),
			Self::Decide(decide) => Self::Decide(Arc::clone(decide)),
		}
	}
}

impl<E> RetryDelay<E> {
	/// Decide per failure; the closure receives the failures so far and the
	/// error that just occurred.
	pub fn decide(decide: impl Fn(u32, &E) -> Duration + Send + Sync + 'static) -> Self {
		Self::Decide(Arc::new(decide))
	}

	/// The delay before the retry that follows `failure_count` failures.
	#[must_use]
	pub fn delay(&self, failure_count: u32, error: &E) -> Duration {
		match self {
			Self::Fixed(delay) => *delay,
			Self::Backoff(backoff) => backoff.delay(failure_count),
			Self::Decide(decide) => decide(failure_count, error),
		}
	}
}

impl<E> Default for RetryDelay<E> {
	fn default() -> Self {
		Self::Backoff(Backoff::default())
	}
}

impl<E> fmt::Debug for RetryDelay<E> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Fixed(delay) => f.debug_tuple("Fixed").field(delay).finish(),
			Self::Backoff(backoff) => f.debug_tuple("Backoff").field(backoff).finish(),
			Self::Decide(_) => f.write_str("Decide(..)"),
		}
	}
}

impl<E> From<Duration> for RetryDelay<E> {
	fn from(delay: Duration) -> Self {
		Self::Fixed(delay)
	}
}

impl<E> From<Backoff> for RetryDelay<E> {
	fn from(backoff: Backoff) -> Self {
		Self::Backoff(backoff)
	}
}
