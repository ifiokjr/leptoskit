#![cfg(feature = "app")]
#![allow(
	unsafe_code,
	reason = "these independent C ABI controls exercise actual foreign ownership contracts"
)]

#[path = "app_services_independent/foreign.rs"]
mod foreign;
#[path = "app_services_independent/support.rs"]
mod support;

use std::cell::RefCell;
use std::mem::MaybeUninit;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::task::Poll;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge_ffi::app::ClosingApp;
use leptoskit_bridge_ffi::app::ServiceWake;
use leptoskit_bridge_ffi::app::lk_app_next_request;
use leptoskit_bridge_ffi::lk_buffer_release;
use support::State;
use support::deferred;
use support::poll;

const METHOD: MethodId = MethodId(9);

struct Payload(Arc<AtomicUsize>);

impl AsRef<[u8]> for Payload {
	fn as_ref(&self) -> &[u8] {
		b"owned until actual joined error"
	}
}

impl Drop for Payload {
	fn drop(&mut self) {
		self.0.fetch_add(1, Ordering::AcqRel);
	}
}

#[test]
fn moved_owner_skips_cancelled_routed_request_and_preserves_both_identities() {
	let state = Arc::new(State::default());
	let mut app = deferred(&state, false);
	let queue = Rc::new(RefCell::new(Vec::<ClosingApp>::new()));
	let scheduler = Rc::clone(&queue);
	app.activate_service(
		ServiceWake::new(|| {}),
		Rc::new(move |owner| scheduler.borrow_mut().push(owner)),
	)
	.unwrap();
	let identities = app.with_host(|host| (host.identity(), host.plugins().identity()));
	let mut moved = Box::new(app);

	let (cancelled, live) = moved.with_host(|host| {
		let bridge = host.plugins();
		let plugin = bridge.register("independent", [METHOD]).unwrap();
		bridge.grant(plugin, METHOD, true).unwrap();
		let first = bridge
			.submit(plugin, METHOD, Bytes::from(&b"cancel"[..]))
			.unwrap();
		let second = bridge
			.submit(plugin, METHOD, Bytes::from(&b"live"[..]))
			.unwrap();
		(first, second)
	});
	moved.poll_service().unwrap();
	moved
		.with_host(|host| host.plugins().cancel(cancelled))
		.unwrap();
	assert_routed_request(&mut moved, live.0);

	moved.poll_service().unwrap();
	assert_eq!(moved.next_cancelled(), Some(cancelled));
	assert_eq!(
		identities,
		moved.with_host(|host| (host.identity(), host.plugins().identity()))
	);
	state.ready.store(true, Ordering::Release);
	assert_eq!(poll(&mut moved.close()), Poll::Ready(Ok(())));
	assert!(queue.borrow().is_empty());
}

fn assert_routed_request(app: &mut leptoskit_bridge_ffi::app::App, id: u64) {
	let mut request = MaybeUninit::uninit();

	// SAFETY: App and output are exclusively borrowed. The returned payload
	// remains valid until its unique buffer owner is released below.
	unsafe {
		assert_eq!(lk_app_next_request(app, request.as_mut_ptr()), 1);
		let request = request.assume_init();
		assert_eq!(request.id, id);
		assert_eq!(
			std::slice::from_raw_parts(request.payload.data, request.payload.len),
			b"live"
		);
		lk_buffer_release(request.payload);
	}
}

#[test]
fn repeated_cancelled_close_retains_interpreter_until_actual_joined_error() {
	let state = Arc::new(State::default());
	let mut app = deferred(&state, true);
	let queue = Rc::new(RefCell::new(Vec::<ClosingApp>::new()));
	let scheduler = Rc::clone(&queue);
	let dropped = Arc::new(AtomicUsize::new(0));
	app.activate_service(
		ServiceWake::new(|| {}),
		Rc::new(move |owner| scheduler.borrow_mut().push(owner)),
	)
	.unwrap();
	app.with_host(|host| {
		let bridge = host.plugins();
		let plugin = bridge.register("retained", [METHOD]).unwrap();
		bridge.grant(plugin, METHOD, true).unwrap();
		bridge
			.submit(
				plugin,
				METHOD,
				Bytes::from_owner(Payload(Arc::clone(&dropped))),
			)
			.unwrap();
	});
	let mut owner = app.close();

	for _ in 0..3 {
		assert_eq!(poll(&mut owner), Poll::Pending);
		assert_eq!(dropped.load(Ordering::Acquire), 0);
		drop(owner);
		assert_eq!(queue.borrow().len(), 1);
		owner = queue.borrow_mut().pop().unwrap();
	}

	assert_eq!(state.closed.load(Ordering::Acquire), 1);
	state.ready.store(true, Ordering::Release);
	assert_eq!(poll(&mut owner), Poll::Ready(Err(Error::Native)));
	assert_eq!(dropped.load(Ordering::Acquire), 1);
	assert_eq!(state.joined.load(Ordering::Acquire), 1);
	assert_eq!(poll(&mut owner), Poll::Ready(Err(Error::Native)));
	state.waker.lock().unwrap().take();
}

#[test]
fn concurrent_notifications_coalesce_again_only_after_owner_poll() {
	let state = Arc::new(State::default());
	let notifications = Arc::new(AtomicUsize::new(0));
	let notify = Arc::clone(&notifications);
	let mut app = deferred(&state, false);
	app.activate_service(
		ServiceWake::new(move || {
			notify.fetch_add(1, Ordering::AcqRel);
		}),
		Rc::new(|_| panic!("this control explicitly retains close")),
	)
	.unwrap();

	for expected in 1..=3 {
		app.poll_service().unwrap();
		let waker = state.waker.lock().unwrap().clone().unwrap();
		std::thread::scope(|scope| {
			for _ in 0..8 {
				let waker = waker.clone();
				scope.spawn(move || waker.wake());
			}
		});
		assert_eq!(notifications.load(Ordering::Acquire), expected);
	}

	let late = state.waker.lock().unwrap().take().unwrap();
	state.ready.store(true, Ordering::Release);
	assert_eq!(poll(&mut app.close()), Poll::Ready(Ok(())));
	late.wake();
	assert_eq!(notifications.load(Ordering::Acquire), 3);
	state.waker.lock().unwrap().take();
}
