use std::sync::Arc;
use std::time::Duration;

use reactive_graph::owner::on_cleanup;
use reactive_graph::prelude::*;
use reactive_graph::signal::ReadSignal;
use reactive_graph::signal::RwSignal;
use reactive_graph::wrappers::read::Signal;

use crate::Action;
use crate::watch_with_options;

/// A host-provided monotonic clock, measured from any fixed origin.
///
/// Advance the signal from your event loop, animation frame, server scheduler,
/// or test. Time must never go backwards. Hooks schedule no work on their own.
/// Using one signal lets an application drive all its timers with one host
/// timer.
pub type Clock = Signal<Duration>;

/// Restartable one-shot timer controls.
#[derive(Clone)]
pub struct UseTimeoutFnReturn {
	/// Whether a deadline is scheduled.
	pub is_pending: Signal<bool>,
	/// Start or restart from the clock's current value.
	pub start: Action,
	/// Cancel the deadline.
	pub stop: Action,
}

/// Run once on the first clock update at or after the deadline.
///
/// Starts immediately. A zero delay fires on the next clock change. Stopping,
/// restarting, or owner cleanup cancels the old deadline. Deadline arithmetic
/// saturates at `Duration::MAX`.
pub fn use_timeout_fn(
	clock: impl Into<Clock>,
	callback: impl Fn() + Send + Sync + 'static,
	delay: Duration,
) -> UseTimeoutFnReturn {
	let clock = clock.into();
	let deadline = RwSignal::new(Some(clock.get_untracked().saturating_add(delay)));
	watch_with_options(
		move || clock.get(),
		move |now, _| {
			if deadline.get_untracked().is_some_and(|at| *now >= at) {
				deadline.set(None);
				callback();
			}
		},
		false,
	);
	on_cleanup(move || {
		deadline.try_set(None);
	});
	UseTimeoutFnReturn {
		is_pending: Signal::derive(move || deadline.with(Option::is_some)),
		start: Arc::new(move || deadline.set(Some(clock.get_untracked().saturating_add(delay)))),
		stop: Arc::new(move || deadline.set(None)),
	}
}

/// Repeating timer controls.
#[derive(Clone)]
pub struct UseIntervalFnReturn {
	/// Whether the interval is running.
	pub is_active: Signal<bool>,
	/// Cancel future ticks.
	pub pause: Action,
	/// Restart a full interval from the current clock value.
	pub resume: Action,
}

/// Run at most once per host clock update, coalescing missed intervals.
///
/// Starts immediately, with the first callback after `interval`. Resuming
/// starts a fresh interval. A zero interval runs once per clock change. There
/// is no catch-up loop after an application resumes from the background.
pub fn use_interval_fn(
	clock: impl Into<Clock>,
	callback: impl Fn() + Send + Sync + 'static,
	interval: Duration,
) -> UseIntervalFnReturn {
	let clock = clock.into();
	let deadline = RwSignal::new(Some(clock.get_untracked().saturating_add(interval)));
	watch_with_options(
		move || clock.get(),
		move |now, _| {
			if deadline.get_untracked().is_some_and(|at| *now >= at) {
				deadline.set(Some(now.saturating_add(interval)));
				callback();
			}
		},
		false,
	);
	on_cleanup(move || {
		deadline.try_set(None);
	});
	UseIntervalFnReturn {
		is_active: Signal::derive(move || deadline.with(Option::is_some)),
		pause: Arc::new(move || deadline.set(None)),
		resume: Arc::new(move || {
			deadline.set(Some(clock.get_untracked().saturating_add(interval)));
		}),
	}
}

/// Count interval ticks. Missed ticks are coalesced, and the count saturates.
pub fn use_interval(clock: impl Into<Clock>, interval: Duration) -> ReadSignal<u64> {
	let count = RwSignal::new(0_u64);
	use_interval_fn(
		clock,
		move || count.update(|n| *n = n.saturating_add(1)),
		interval,
	);
	count.read_only()
}

/// A trailing-edge debounced function. Repeated calls restart the deadline.
/// The callback has no arguments: capture signals and read them when it runs.
pub fn use_debounce_fn(
	clock: impl Into<Clock>,
	callback: impl Fn() + Send + Sync + 'static,
	delay: Duration,
) -> Action {
	let timer = use_timeout_fn(clock, callback, delay);
	(timer.stop)();
	timer.start
}

/// A leading-edge throttled function; calls inside the interval are dropped.
/// Zero duration permits every call. No trailing invocation is scheduled.
pub fn use_throttle_fn(
	clock: impl Into<Clock>,
	callback: impl Fn() + Send + Sync + 'static,
	interval: Duration,
) -> Action {
	let clock = clock.into();
	let next = RwSignal::new(Duration::ZERO);
	Arc::new(move || {
		let now = clock.get_untracked();
		if now >= next.get_untracked() {
			next.set(now.saturating_add(interval));
			callback();
		}
	})
}

/// Debounce changes to a signal, retaining the initial value immediately.
pub fn signal_debounced<T>(
	source: impl Get<Value = T> + Send + Sync + 'static,
	clock: impl Into<Clock>,
	delay: Duration,
) -> ReadSignal<T>
where
	T: Clone + PartialEq + Send + Sync + 'static,
{
	let source = Signal::derive(move || source.get());
	let output = RwSignal::new(source.get_untracked());
	let trigger = use_debounce_fn(clock, move || output.set(source.get_untracked()), delay);
	watch_with_options(move || source.get(), move |_, _| trigger(), false);
	output.read_only()
}

/// Throttle changes to a signal on the leading edge, without a trailing update.
pub fn signal_throttled<T>(
	source: impl Get<Value = T> + Send + Sync + 'static,
	clock: impl Into<Clock>,
	interval: Duration,
) -> ReadSignal<T>
where
	T: Clone + PartialEq + Send + Sync + 'static,
{
	let source = Signal::derive(move || source.get());
	let output = RwSignal::new(source.get_untracked());
	let trigger = use_throttle_fn(clock, move || output.set(source.get_untracked()), interval);
	watch_with_options(move || source.get(), move |_, _| trigger(), false);
	output.read_only()
}
