use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use reactive_graph::effect::ImmediateEffect;
use reactive_graph::graph::untrack;
use reactive_graph::owner::on_cleanup;
use reactive_graph::prelude::*;
use reactive_graph::signal::RwSignal;
use reactive_graph::wrappers::read::Signal;

use crate::Action;

/// The latest uninvoked observation: current value and its predecessor.
type PendingChange<T> = Arc<Mutex<Option<(T, Option<T>)>>>;
use crate::time::Clock;
use crate::use_debounce_fn;
use crate::use_throttle_fn;

/// Controls for a synchronous watcher.
#[derive(Clone)]
pub struct WatchPausableReturn {
	/// Whether callbacks are enabled. Stopped watchers are never active.
	pub is_active: Signal<bool>,
	/// Suppress callbacks while still tracking the source.
	pub pause: Action,
	/// Enable callbacks for future changes, without replaying paused changes.
	pub resume: Action,
	/// Permanently unsubscribe. Calling this repeatedly is harmless.
	pub stop: Action,
}

/// Watch a source synchronously, with pause/resume and owner-scoped cleanup.
///
/// The callback receives the current and previous observed values. It runs
/// untracked, so its signal reads do not become dependencies. Equal values are
/// ignored. `immediate` also calls it on creation, with no previous value.
/// Source functions should be pure; callbacks must not recursively mutate their
/// own source. No executor or `effects` feature is required, including on SSR.
pub fn watch_pausable<T>(
	source: impl Fn() -> T + Send + Sync + 'static,
	callback: impl Fn(&T, Option<&T>) + Send + Sync + 'static,
	immediate: bool,
) -> WatchPausableReturn
where
	T: PartialEq + Clone + Send + Sync + 'static,
{
	let active = RwSignal::new(true);
	let previous = Mutex::new(None);
	let effect = ImmediateEffect::new_isomorphic(move || {
		let value = source();
		let old = previous.lock().replace(value.clone());
		if active.get_untracked() && old.as_ref() != Some(&value) && (immediate || old.is_some()) {
			untrack(|| callback(&value, old.as_ref()));
		}
	});
	let effect = Arc::new(Mutex::new(Some(effect)));
	let stopped = Arc::clone(&effect);
	let stop: Action = Arc::new(move || {
		stopped.lock().take();
		active.try_set(false);
	});
	let cleanup = Arc::clone(&stop);
	on_cleanup(move || cleanup());
	WatchPausableReturn {
		is_active: active.into(),
		pause: Arc::new(move || {
			active.try_set(false);
		}),
		resume: Arc::new(move || {
			if effect.lock().is_some() {
				active.try_set(true);
			}
		}),
		stop,
	}
}

/// Watch with an optional initial callback; returns a permanent stop action.
pub fn watch_with_options<T>(
	source: impl Fn() -> T + Send + Sync + 'static,
	callback: impl Fn(&T, Option<&T>) + Send + Sync + 'static,
	immediate: bool,
) -> Action
where
	T: PartialEq + Clone + Send + Sync + 'static,
{
	watch_pausable(source, callback, immediate).stop
}

/// Call whenever a boolean becomes true (including its initial true value).
pub fn whenever(
	source: impl Into<Signal<bool>>,
	callback: impl Fn() + Send + Sync + 'static,
) -> Action {
	let source = source.into();
	watch_with_options(
		move || source.get(),
		move |value, _| {
			if *value {
				callback();
			}
		},
		true,
	)
}

/// Synchronize two signals in both directions, initially preferring `left`.
///
/// Equality checks prevent feedback. Returns an action stopping both watchers.
/// Each pair belongs to the current owner, not a process-global scope.
pub fn sync_signal<T>(left: RwSignal<T>, right: RwSignal<T>) -> Action
where
	T: PartialEq + Clone + Send + Sync + 'static,
{
	let forward = watch_with_options(
		move || left.get(),
		move |value, _| {
			if right.get_untracked() != *value {
				right.set(value.clone());
			}
		},
		true,
	);
	let backward = watch_with_options(
		move || right.get(),
		move |value, _| {
			if left.get_untracked() != *value {
				left.set(value.clone());
			}
		},
		false,
	);
	Arc::new(move || {
		forward();
		backward();
	})
}

/// Watch a source and invoke the callback on the trailing edge of a debounce
/// window.
///
/// Rapid changes extend the window; the callback runs once, after `delay`
/// passes without further changes, with the latest and previous values.
/// Owner cleanup cancels a pending invocation.
pub fn watch_debounced<T>(
	source: impl Fn() -> T + Send + Sync + 'static,
	callback: impl Fn(&T, Option<&T>) + Send + Sync + 'static,
	clock: impl Into<Clock>,
	delay: Duration,
) -> Action
where
	T: PartialEq + Clone + Send + Sync + 'static,
{
	let latest: PendingChange<T> = Arc::new(Mutex::new(None));
	let pending = Arc::clone(&latest);
	let invoke = use_debounce_fn(
		clock,
		move || {
			if let Some((value, previous)) = pending.lock().take() {
				callback(&value, previous.as_ref());
			}
		},
		delay,
	);
	let queue = Arc::clone(&latest);
	watch_with_options(
		source,
		move |value, old| {
			*queue.lock() = Some((value.clone(), old.cloned()));
			invoke();
		},
		false,
	)
}

/// Watch a source and invoke the callback on the leading edge of a throttle
/// window.
///
/// The callback runs for the first change in each `interval`; further changes
/// inside the window are dropped without a trailing invocation, and the last
/// dropped value is never observed by the callback.
pub fn watch_throttled<T>(
	source: impl Fn() -> T + Send + Sync + 'static,
	callback: impl Fn(&T, Option<&T>) + Send + Sync + 'static,
	clock: impl Into<Clock>,
	interval: Duration,
) -> Action
where
	T: PartialEq + Clone + Send + Sync + 'static,
{
	let latest: PendingChange<T> = Arc::new(Mutex::new(None));
	let pending = Arc::clone(&latest);
	let invoke = use_throttle_fn(
		clock,
		move || {
			if let Some((value, previous)) = pending.lock().take() {
				callback(&value, previous.as_ref());
			}
		},
		interval,
	);
	let queue = Arc::clone(&latest);
	watch_with_options(
		source,
		move |value, old| {
			*queue.lock() = Some((value.clone(), old.cloned()));
			invoke();
		},
		false,
	)
}
