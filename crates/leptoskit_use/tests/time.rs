use std::sync::Arc;
use std::time::Duration;

use leptoskit_use::*;
use parking_lot::Mutex;
use reactive_graph::effect::ImmediateEffect;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;
use reactive_graph::signal::ArcRwSignal;
use reactive_graph::signal::RwSignal;

fn ms(n: u64) -> Duration {
	Duration::from_millis(n)
}

#[test]
fn timeout_cancels_restarts_and_fires_only_once() {
	Owner::new().with(|| {
		let clock = RwSignal::new(Duration::ZERO);
		let calls = Arc::new(Mutex::new(0));
		let log = Arc::clone(&calls);
		let timer = use_timeout_fn(clock, move || *log.lock() += 1, ms(10));
		assert!(timer.is_pending.get());
		clock.set(ms(5));
		assert_eq!(*calls.lock(), 0);
		(timer.start)();
		clock.set(ms(10));
		assert_eq!(*calls.lock(), 0);
		clock.set(ms(15));
		assert_eq!(*calls.lock(), 1);
		assert!(!timer.is_pending.get());
		clock.set(ms(20));
		assert_eq!(*calls.lock(), 1);
		(timer.start)();
		(timer.stop)();
		(timer.stop)();
		clock.set(ms(40));
		assert_eq!(*calls.lock(), 1);
		(timer.start)();
		clock.set(ms(50));
		assert_eq!(*calls.lock(), 2);
	});
}

#[test]
fn interval_coalesces_missed_ticks_and_resume_restarts_deadline() {
	Owner::new().with(|| {
		let clock = RwSignal::new(Duration::ZERO);
		let calls = Arc::new(Mutex::new(0));
		let log = Arc::clone(&calls);
		let timer = use_interval_fn(clock, move || *log.lock() += 1, ms(10));
		let count = use_interval(clock, ms(10));
		assert!(timer.is_active.get());
		clock.set(ms(9));
		assert_eq!(*calls.lock(), 0);
		clock.set(ms(10));
		clock.set(ms(100));
		assert_eq!(*calls.lock(), 2);
		assert_eq!(count.get(), 2);
		(timer.pause)();
		assert!(!timer.is_active.get());
		clock.set(ms(200));
		assert_eq!(*calls.lock(), 2);
		(timer.resume)();
		assert!(timer.is_active.get());
		clock.set(ms(209));
		assert_eq!(*calls.lock(), 2);
		clock.set(ms(210));
		assert_eq!(*calls.lock(), 3);
	});
}

#[test]
fn zero_delays_and_saturating_deadlines_have_defined_behavior() {
	Owner::new().with(|| {
		let clock = RwSignal::new(Duration::ZERO);
		let calls = Arc::new(Mutex::new(0));
		let log = Arc::clone(&calls);
		use_timeout_fn(clock, move || *log.lock() += 1, Duration::ZERO);
		let count = use_interval(clock, Duration::ZERO);
		assert_eq!(*calls.lock(), 0);
		assert_eq!(count.get(), 0);
		clock.set(ms(1));
		assert_eq!(*calls.lock(), 1);
		assert_eq!(count.get(), 1);
		clock.set(ms(2));
		assert_eq!(count.get(), 2);
		clock.set(
			Duration::MAX
				.checked_sub(ms(1))
				.expect("Duration::MAX is longer than a millisecond"),
		);
		let log = Arc::clone(&calls);
		let timer = use_timeout_fn(clock, move || *log.lock() += 1, ms(10));
		clock.set(Duration::MAX);
		assert_eq!(*calls.lock(), 2);
		assert!(!timer.is_pending.get());
	});
}

#[test]
fn owner_cleanup_cancels_timers_even_when_host_clock_survives() {
	let clock = ArcRwSignal::new(Duration::ZERO);
	let calls = Arc::new(Mutex::new(0));
	let owner = Owner::new();
	owner.with(|| {
		let log = Arc::clone(&calls);
		use_timeout_fn(clock.clone(), move || *log.lock() += 1, ms(10));
		let log = Arc::clone(&calls);
		use_interval_fn(clock.clone(), move || *log.lock() += 1, ms(10));
	});
	owner.cleanup();
	clock.set(ms(10));
	assert_eq!(*calls.lock(), 0);
}

#[test]
fn debounce_waits_for_last_call_and_throttle_drops_intermediate_calls() {
	Owner::new().with(|| {
		let clock = RwSignal::new(Duration::ZERO);
		let debounced = Arc::new(Mutex::new(0));
		let log = Arc::clone(&debounced);
		let debounce = use_debounce_fn(clock, move || *log.lock() += 1, ms(10));
		clock.set(ms(20));
		assert_eq!(*debounced.lock(), 0);
		debounce();
		clock.set(ms(25));
		debounce();
		clock.set(ms(30));
		assert_eq!(*debounced.lock(), 0);
		clock.set(ms(35));
		assert_eq!(*debounced.lock(), 1);
		clock.set(ms(50));
		assert_eq!(*debounced.lock(), 1);
		let throttled = Arc::new(Mutex::new(0));
		let log = Arc::clone(&throttled);
		let throttle = use_throttle_fn(clock, move || *log.lock() += 1, ms(10));
		throttle();
		throttle();
		clock.set(ms(59));
		throttle();
		assert_eq!(*throttled.lock(), 1);
		clock.set(ms(60));
		throttle();
		assert_eq!(*throttled.lock(), 2);
		let log = Arc::clone(&throttled);
		let zero = use_throttle_fn(clock, move || *log.lock() += 1, Duration::ZERO);
		zero();
		zero();
		assert_eq!(*throttled.lock(), 4);
	});
}

#[test]
fn filtered_signals_start_with_source_and_follow_timing_policy() {
	Owner::new().with(|| {
		let clock = RwSignal::new(Duration::ZERO);
		let source = RwSignal::new("initial");
		let debounced = signal_debounced(source, clock, ms(10));
		let throttled = signal_throttled(source, clock, ms(10));
		assert_eq!(debounced.get(), "initial");
		assert_eq!(throttled.get(), "initial");
		source.set("first");
		assert_eq!(throttled.get(), "first");
		clock.set(ms(5));
		source.set("second");
		clock.set(ms(10));
		assert_eq!(debounced.get(), "initial");
		assert_eq!(throttled.get(), "first");
		clock.set(ms(15));
		assert_eq!(debounced.get(), "second");
		source.set("third");
		assert_eq!(throttled.get(), "third");
		clock.set(ms(25));
		assert_eq!(debounced.get(), "third");
	});
}

#[test]
fn timestamp_tracks_elapsed_clock_time_since_creation() {
	Owner::new().with(|| {
		let clock = RwSignal::new(ms(100));
		let stamp = use_timestamp(clock);
		assert_eq!(stamp.get(), Duration::ZERO);
		clock.set(ms(150));
		assert_eq!(stamp.get(), ms(50));
		clock.set(ms(50));
		assert_eq!(stamp.get(), Duration::ZERO);
	});
}

#[test]
fn filtered_signals_do_not_notify_when_the_value_comes_back() {
	Owner::new().with(|| {
		let clock = RwSignal::new(Duration::ZERO);
		let source = RwSignal::new(1);
		let debounced = signal_debounced(source, clock, ms(10));
		let throttled = signal_throttled(source, clock, ms(10));
		let notified = Arc::new(Mutex::new((0, 0)));
		let log = Arc::clone(&notified);
		let _debounced = ImmediateEffect::new_isomorphic(move || {
			debounced.track();
			log.lock().0 += 1;
		});
		let log = Arc::clone(&notified);
		let _throttled = ImmediateEffect::new_isomorphic(move || {
			throttled.track();
			log.lock().1 += 1;
		});
		assert_eq!(*notified.lock(), (1, 1), "each effect ran once on creation");

		// The leading edge lets 2 through; 3 lands inside the interval and is
		// dropped; 2 again after it is the value the output already holds.
		source.set(2);
		assert_eq!(throttled.get_untracked(), 2);
		clock.set(ms(5));
		source.set(3);
		clock.set(ms(10));
		source.set(2);
		assert_eq!(throttled.get_untracked(), 2);
		assert_eq!(notified.lock().1, 2, "only the change to 2 notified");

		// The source settles back on the value the debounced output started
		// with, so the trailing write changes nothing.
		source.set(1);
		clock.set(ms(25));
		assert_eq!(debounced.get_untracked(), 1);
		assert_eq!(notified.lock().0, 1, "an unchanged value does not notify");
	});
}
