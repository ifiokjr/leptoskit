//! Reactive location fixes over the plugin's sources.
//!
//! [`use_geolocation`] publishes the first fix, then waits for strictly newer
//! timestamps until the reactive owner is disposed or a source call fails.
//! The host executor owns the driving future; nothing is spawned internally.

use std::future::Future;
use std::pin::Pin;

use reactive_graph::prelude::*;
use reactive_graph::signal::ReadSignal;
use reactive_graph::signal::RwSignal;

use crate::Fix;
use crate::LocationError;
use crate::Options;

/// One source operation, polled by the caller's executor.
type SourceFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// The future that keeps a location signal current.
pub type GeolocationTask = Pin<Box<dyn Future<Output = Result<(), LocationError>>>>;

mod private {
	/// Seals [`Source`](super::Source) to this crate's implementations.
	pub trait Sealed {}
}

/// Fix sources usable with [`use_geolocation`]: the interpreter's client,
/// the browser API, and shared handles to any source. Sealed; test doubles
/// live in this crate.
pub trait Source: private::Sealed + Send + Sync + 'static {
	/// Returns a fresh or age-tolerated fix; reads never prompt.
	fn current(&self, options: Options) -> SourceFuture<'_, Result<Fix, LocationError>>;
	/// Waits for a fix strictly newer than `after_timestamp_ms`.
	fn next(
		&self,
		options: Options,
		after_timestamp_ms: f64,
	) -> SourceFuture<'_, Result<Fix, LocationError>>;
}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
impl private::Sealed for crate::guest::Location {}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
impl Source for crate::guest::Location {
	fn current(&self, options: Options) -> SourceFuture<'_, Result<Fix, LocationError>> {
		let client = *self;
		Box::pin(async move { client.current(options).await })
	}

	fn next(
		&self,
		options: Options,
		after_timestamp_ms: f64,
	) -> SourceFuture<'_, Result<Fix, LocationError>> {
		let client = *self;
		Box::pin(async move { client.next(options, after_timestamp_ms).await })
	}
}

/// Browser Geolocation API source; permission denials surface as errors.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
#[derive(Debug, Clone, Copy)]
pub struct WebLocation;

#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
impl private::Sealed for WebLocation {}

#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
impl Source for WebLocation {
	fn current(&self, options: Options) -> SourceFuture<'_, Result<Fix, LocationError>> {
		Box::pin(crate::web::current(options))
	}

	fn next(
		&self,
		options: Options,
		after_timestamp_ms: f64,
	) -> SourceFuture<'_, Result<Fix, LocationError>> {
		Box::pin(crate::web::next(options, after_timestamp_ms))
	}
}

impl<S: Source> private::Sealed for std::sync::Arc<S> {}

impl<S: Source> Source for std::sync::Arc<S> {
	fn current(&self, options: Options) -> SourceFuture<'_, Result<Fix, LocationError>> {
		(**self).current(options)
	}

	fn next(
		&self,
		options: Options,
		after_timestamp_ms: f64,
	) -> SourceFuture<'_, Result<Fix, LocationError>> {
		(**self).next(options, after_timestamp_ms)
	}
}

/// Reactive location state.
pub struct UseGeolocationReturn {
	/// `None` until the first fix resolves; later failures keep the last
	/// published fix.
	pub value: ReadSignal<Option<Fix>>,
	/// Spawn once on the host executor. Completes with `Ok` when the owner is
	/// disposed, or with the first source error.
	pub task: GeolocationTask,
}

/// Publishes location fixes into a signal inside a Leptos owner.
///
/// The task performs one read, then repeatedly waits for fixes with strictly
/// newer timestamps, passing the last published timestamp back to the source.
/// Reads never prompt for permission; request access through the plugin's
/// permission calls first. Owner disposal ends the task cleanly once the
/// in-flight operation resolves, and dropping the task stops native tracking.
pub fn use_geolocation(source: impl Source, options: Options) -> UseGeolocationReturn {
	let value = RwSignal::new(None);
	let task = Box::pin(async move {
		let first = source.current(options).await?;
		if value.try_set(Some(first)).is_some() {
			return Ok(());
		}

		let mut after_timestamp_ms = first.timestamp_ms;
		loop {
			if value.is_disposed() {
				return Ok(());
			}

			let next = source.next(options, after_timestamp_ms).await?;
			if value.try_set(Some(next)).is_some() {
				return Ok(());
			}

			after_timestamp_ms = next.timestamp_ms;
		}
	});
	UseGeolocationReturn {
		value: value.read_only(),
		task,
	}
}

#[cfg(all(not(target_arch = "wasm32"), test))]
mod tests {
	use std::collections::VecDeque;
	use std::pin::Pin;
	use std::sync::Arc;
	use std::sync::Mutex;
	use std::task::Context;
	use std::task::Poll;
	use std::task::Waker;

	use futures::executor::block_on;
	use futures::task::noop_waker;
	use reactive_graph::owner::Owner;
	use reactive_graph::prelude::Get;

	use super::Source;
	use super::use_geolocation;
	use crate::Fix;
	use crate::LocationError;
	use crate::Options;
	use crate::Provider;

	/// The timeout a reactive source waits for each fix.
	const SAMPLE_TIMEOUT_MS: u32 = 30_000;

	fn options() -> Options {
		Options::new(false, SAMPLE_TIMEOUT_MS, 0).unwrap()
	}

	fn fix(timestamp_ms: f64, latitude: f64) -> Fix {
		Fix {
			timestamp_ms,
			latitude,
			longitude: 0.0,
			accuracy_m: 10.0,
			altitude_m: None,
			altitude_accuracy_m: None,
			heading_degrees: None,
			speed_mps: None,
			provider: Provider::Unknown,
			simulated: None,
		}
	}

	/// Yields queued results in order; an empty queue reports `TimedOut`.
	struct Fake {
		calls: Mutex<VecDeque<Result<Fix, LocationError>>>,
		watched_from: Mutex<Vec<f64>>,
		delay_first_wait: std::sync::atomic::AtomicBool,
	}

	impl Fake {
		fn new(calls: Vec<Result<Fix, LocationError>>) -> Self {
			Self {
				calls: Mutex::new(calls.into()),
				watched_from: Mutex::new(Vec::new()),
				delay_first_wait: std::sync::atomic::AtomicBool::new(false),
			}
		}
	}

	/// Pends once when asked, then yields. Lets tests act between polls.
	struct PendingOnce {
		result: Result<Fix, LocationError>,
		delay: bool,
	}

	impl Future for PendingOnce {
		type Output = Result<Fix, LocationError>;

		fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
			if self.delay {
				self.delay = false;
				context.waker().wake_by_ref();

				return Poll::Pending;
			}

			Poll::Ready(self.result)
		}
	}

	impl super::private::Sealed for Fake {}

	impl Source for Fake {
		fn current(
			&self,
			_options: Options,
		) -> super::SourceFuture<'_, Result<Fix, LocationError>> {
			Box::pin(std::future::ready(
				self.calls
					.lock()
					.unwrap()
					.pop_front()
					.unwrap_or(Err(LocationError::TimedOut)),
			))
		}

		fn next(
			&self,
			_options: Options,
			after_timestamp_ms: f64,
		) -> super::SourceFuture<'_, Result<Fix, LocationError>> {
			self.watched_from.lock().unwrap().push(after_timestamp_ms);
			let result = self
				.calls
				.lock()
				.unwrap()
				.pop_front()
				.unwrap_or(Err(LocationError::TimedOut));
			let delay = self
				.delay_first_wait
				.swap(false, std::sync::atomic::Ordering::Relaxed);
			Box::pin(PendingOnce { result, delay })
		}
	}

	/// Two measured coordinates in the order the fake source reports them.
	const FIRST_TIMESTAMP_MS: f64 = 1000.0;
	const FIRST_LATITUDE: f64 = 51.0;
	const SECOND_TIMESTAMP_MS: f64 = 5000.0;
	const SECOND_LATITUDE: f64 = 51.1;
	const EARLIER_TIMESTAMP_MS: f64 = 2000.0;
	const OTHER_LATITUDE: f64 = 51.2;

	#[test]
	fn publishes_fixes_and_chains_timestamps_until_the_source_fails() {
		let owner = Owner::new();
		let source = Arc::new(Fake::new(vec![
			Ok(fix(FIRST_TIMESTAMP_MS, FIRST_LATITUDE)),
			Ok(fix(SECOND_TIMESTAMP_MS, SECOND_LATITUDE)),
			Err(LocationError::TimedOut),
		]));
		let hook = owner.with(|| use_geolocation(Arc::clone(&source), options()));
		assert_eq!(hook.value.get(), None);
		assert_eq!(block_on(hook.task), Err(LocationError::TimedOut));
		assert_eq!(
			hook.value.get().map(|fix| fix.latitude),
			Some(SECOND_LATITUDE)
		);
		assert_eq!(
			source.watched_from.lock().unwrap().as_slice(),
			[FIRST_TIMESTAMP_MS, SECOND_TIMESTAMP_MS]
		);
		owner.cleanup();
	}

	#[test]
	fn owner_disposal_during_a_wait_ends_the_task_cleanly() {
		let owner = Owner::new();
		let source = Fake::new(vec![
			Ok(fix(FIRST_TIMESTAMP_MS, FIRST_LATITUDE)),
			Ok(fix(EARLIER_TIMESTAMP_MS, OTHER_LATITUDE)),
		]);
		source
			.delay_first_wait
			.store(true, std::sync::atomic::Ordering::Relaxed);
		let hook = owner.with(|| use_geolocation(source, options()));
		let mut task = hook.task;
		let waker: Waker = noop_waker();
		let mut context = Context::from_waker(&waker);
		assert!(task.as_mut().poll(&mut context).is_pending());

		owner.cleanup();
		assert_eq!(task.as_mut().poll(&mut context), Poll::Ready(Ok(())));
	}
}
