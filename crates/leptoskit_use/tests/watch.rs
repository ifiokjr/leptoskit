use std::sync::Arc;

use leptoskit_use::*;
use parking_lot::Mutex;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;
use reactive_graph::signal::ArcRwSignal;
use reactive_graph::signal::RwSignal;

#[test]
fn watch_observes_changes_pauses_resumes_and_stops_permanently() {
	Owner::new().with(|| {
		let source = RwSignal::new(1);
		let calls = Arc::new(Mutex::new(vec![]));
		let log = Arc::clone(&calls);
		let watch = watch_pausable(
			move || source.get(),
			move |next, old| log.lock().push((*next, old.copied())),
			true,
		);
		assert_eq!(*calls.lock(), vec![(1, None)]);
		source.set(1);
		source.set(2);
		(watch.pause)();
		assert!(!watch.is_active.get());
		source.set(3);
		(watch.resume)();
		assert!(watch.is_active.get());
		assert_eq!(*calls.lock(), vec![(1, None), (2, Some(1))]);
		source.set(4);
		assert_eq!(calls.lock().last(), Some(&(4, Some(3))));
		(watch.stop)();
		(watch.stop)();
		(watch.resume)();
		assert!(!watch.is_active.get());
		source.set(5);
		assert_eq!(calls.lock().len(), 3);
	});
}

#[test]
fn callback_reads_are_untracked_and_initial_call_is_optional() {
	Owner::new().with(|| {
		let source = RwSignal::new(1);
		let other = RwSignal::new(5);
		let calls = Arc::new(Mutex::new(vec![]));
		let log = Arc::clone(&calls);
		let stop = watch_with_options(
			move || source.get(),
			move |_, old| log.lock().push((old.copied(), other.get())),
			false,
		);
		assert!(calls.lock().is_empty());
		source.set(2);
		other.set(6);
		assert_eq!(*calls.lock(), vec![(Some(1), 5)]);
		stop();
		source.set(3);
		assert_eq!(calls.lock().len(), 1);
	});
}

#[test]
fn owner_cleanup_unsubscribes_and_controls_tolerate_disposal() {
	let source = ArcRwSignal::new(0);
	let read = source.clone();
	let calls = Arc::new(Mutex::new(0));
	let log = Arc::clone(&calls);
	let owner = Owner::new();
	let watch =
		owner.with(|| watch_pausable(move || read.get(), move |_, _| *log.lock() += 1, false));
	source.set(1);
	assert_eq!(*calls.lock(), 1);
	owner.cleanup();
	(watch.pause)();
	(watch.stop)();
	(watch.resume)();
	source.set(2);
	assert_eq!(*calls.lock(), 1);
}

#[test]
fn whenever_runs_on_true_transitions_only() {
	Owner::new().with(|| {
		let source = RwSignal::new(false);
		let calls = Arc::new(Mutex::new(0));
		let log = Arc::clone(&calls);
		let stop = whenever(source, move || *log.lock() += 1);
		source.set(true);
		source.set(true);
		source.set(false);
		source.set(true);
		assert_eq!(*calls.lock(), 2);
		stop();
		source.set(false);
		source.set(true);
		assert_eq!(*calls.lock(), 2);
		let log = Arc::clone(&calls);
		whenever(source, move || *log.lock() += 1);
		assert_eq!(*calls.lock(), 3);
	});
}

#[test]
fn sync_signal_initializes_and_propagates_both_directions_without_feedback() {
	Owner::new().with(|| {
		let left = RwSignal::new("left");
		let right = RwSignal::new("right");
		let stop = sync_signal(left, right);
		assert_eq!(right.get(), "left");
		left.set("new left");
		assert_eq!(right.get(), "new left");
		right.set("new right");
		assert_eq!(left.get(), "new right");
		left.set("new right");
		stop();
		left.set("stopped");
		assert_eq!(right.get(), "new right");
		let stop_same = sync_signal(left, left);
		left.set("same");
		assert_eq!(left.get(), "same");
		stop_same();
	});
}

#[test]
fn derived_values_notify_leptos_subscribers() {
	Owner::new().with(|| {
		let source = RwSignal::new(vec![2, 1]);
		let sorted = use_sorted(source);
		let seen = Arc::new(Mutex::new(vec![]));
		let log = Arc::clone(&seen);
		watch_with_options(
			move || sorted.get(),
			move |value, _| log.lock().push(value.clone()),
			true,
		);
		source.set(vec![5, 3]);
		assert_eq!(*seen.lock(), vec![vec![1, 2], vec![3, 5]]);
	});
}
