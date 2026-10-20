#![cfg(feature = "app")]
#![allow(unsafe_code)]

use std::mem::MaybeUninit;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;
use leptoskit_bridge_ffi::app::*;
use leptoskit_bridge_ffi::*;

const GUEST: &[u8] = br#"(module
 (memory (export "memory") 1)
 (func (export "leptoskit_main"))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0)
 (func (export "leptoskit_plugins_ready"))
)"#;

struct Echo(Arc<AtomicUsize>);
impl Drop for Echo {
	fn drop(&mut self) {
		self.0.fetch_add(1, Ordering::SeqCst);
	}
}
impl Plugin for Echo {
	fn name(&self) -> &'static str {
		"worker"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[MethodId(0)]
	}

	fn invoke(&mut self, _: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		Ok(payload)
	}
}

unsafe fn queued(drops: Arc<AtomicUsize>) -> (*mut App, NativeRequest, *mut RustWorker) {
	let mut app = Box::new(App::new(GUEST).unwrap());
	let id = app.install_worker(Box::new(Echo(drops))).unwrap();
	let app = Box::into_raw(app);
	// SAFETY: This helper owns the app and every returned request.
	unsafe {
		let call = lk_bridge_submit(lk_app_plugins(app), id.0, 0, b"retained".as_ptr(), 8);
		assert!(call > 0);
		let mut request = MaybeUninit::uninit();
		assert_eq!(lk_app_next_request(app, request.as_mut_ptr()), 1);
		let request = request.assume_init();
		let worker = lk_app_worker(app, &raw const request);
		assert!(!worker.is_null());
		(app, request, worker)
	}
}

#[test]
fn worker_outlives_app_and_request_on_another_thread() {
	let drops = Arc::new(AtomicUsize::new(0));
	// SAFETY: Each ownership handle is consumed exactly once.
	unsafe {
		let (app, request, worker) = queued(Arc::clone(&drops));
		let worker = Box::from_raw(worker);
		lk_buffer_release(request.payload);
		lk_app_destroy(app);
		assert_eq!(drops.load(Ordering::SeqCst), 0);
		std::thread::spawn(move || {
			let reply = lk_worker_execute(Box::into_raw(worker));
			lk_worker_reply_release(reply);
		})
		.join()
		.unwrap();
		assert_eq!(drops.load(Ordering::SeqCst), 1);
	}
}

#[test]
fn worker_completion_moves_original_bytes_without_copying() {
	// SAFETY: App/bridge accesses remain on this thread; worker owns its input.
	unsafe {
		let (app, request, worker) = queued(Arc::new(AtomicUsize::new(0)));
		let original = request.payload.data;
		lk_buffer_release(request.payload);
		let reply = lk_worker_execute(worker);
		assert_eq!(
			lk_worker_complete(lk_app_plugins(app), request.id, reply),
			0
		);
		let mut result = MaybeUninit::uninit();
		assert_eq!(
			lk_bridge_take_result(lk_app_plugins(app), request.id, result.as_mut_ptr()),
			1
		);
		let result = result.assume_init();
		assert_eq!(result.data, original);
		assert_eq!(
			std::slice::from_raw_parts(result.data, result.len),
			b"retained"
		);
		lk_buffer_release(result);
		lk_app_destroy(app);
	}
}

#[test]
fn cancelled_or_closed_apps_consume_worker_replies_safely() {
	// SAFETY: All handles are live and uniquely owned when consumed.
	unsafe {
		let (app, request, worker) = queued(Arc::new(AtomicUsize::new(0)));
		lk_buffer_release(request.payload);
		assert_eq!(lk_bridge_cancel(lk_app_plugins(app), request.id), 0);
		assert_eq!(
			lk_bridge_next_cancelled(lk_app_plugins(app)),
			request.id as i64
		);
		assert_eq!(
			lk_worker_complete(lk_app_plugins(app), request.id, lk_worker_execute(worker)),
			-(Error::InvalidCall as i32)
		);
		lk_app_destroy(app);

		let (app, request, worker) = queued(Arc::new(AtomicUsize::new(0)));
		lk_buffer_release(request.payload);
		lk_app_destroy(app);
		assert_eq!(
			lk_worker_complete(ptr::null_mut(), request.id, lk_worker_execute(worker)),
			-(Error::InvalidArgument as i32)
		);
	}
}

#[test]
fn dropping_unstarted_work_releases_its_plugin_and_payload() {
	let drops = Arc::new(AtomicUsize::new(0));
	// SAFETY: Work is released without executing it.
	unsafe {
		let (app, request, worker) = queued(Arc::clone(&drops));
		lk_app_destroy(app);
		lk_buffer_release(request.payload);
		assert_eq!(drops.load(Ordering::SeqCst), 0);
		lk_worker_release(worker);
		assert_eq!(drops.load(Ordering::SeqCst), 1);
	}
}

struct Panics;
impl Plugin for Panics {
	fn name(&self) -> &'static str {
		"panics"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[MethodId(0)]
	}

	fn invoke(&mut self, _: MethodId, _: Bytes) -> Result<Bytes, Error> {
		panic!("worker failed while mutating its state");
	}
}

#[test]
fn panicking_plugin_returns_failure_and_cannot_reenter_damaged_state() {
	let mut app = Box::new(App::new(GUEST).unwrap());
	let id = app.install_worker(Box::new(Panics)).unwrap();
	// SAFETY: App and all ownership handles remain uniquely owned by this test.
	unsafe {
		for _ in 0..2 {
			let bridge = lk_app_plugins(&raw mut *app);
			let call = lk_bridge_submit(bridge, id.0, 0, ptr::null(), 0);
			let mut request = MaybeUninit::uninit();
			assert_eq!(lk_app_next_request(&raw mut *app, request.as_mut_ptr()), 1);
			let request = request.assume_init();
			let worker = lk_app_worker(&raw mut *app, &raw const request);
			lk_buffer_release(request.payload);
			let reply = lk_worker_execute(worker);
			assert_eq!(lk_worker_complete(bridge, call as u64, reply), 0);
			let mut output = MaybeUninit::uninit();
			assert_eq!(
				lk_bridge_take_result(bridge, call as u64, output.as_mut_ptr()),
				-(Error::Native as i32)
			);
		}
	}
}

struct Blocking {
	started: std::sync::mpsc::Sender<()>,
	release: std::sync::mpsc::Receiver<()>,
	invocations: Arc<AtomicUsize>,
}

impl Plugin for Blocking {
	fn name(&self) -> &'static str {
		"blocking"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[MethodId(0)]
	}

	fn invoke(&mut self, _: MethodId, _: Bytes) -> Result<Bytes, Error> {
		if self.invocations.fetch_add(1, Ordering::SeqCst) == 0 {
			self.started.send(()).unwrap();
			self.release.recv().unwrap();
		}
		Ok(Bytes::new())
	}
}

#[test]
fn cancellation_while_waiting_for_another_invocation_prevents_side_effects() {
	let (started, first_started) = std::sync::mpsc::channel();
	let (release_first, release) = std::sync::mpsc::channel();
	let invocations = Arc::new(AtomicUsize::new(0));
	let mut app = Box::new(App::new(GUEST).unwrap());
	let id = app
		.install_worker(Box::new(Blocking {
			started,
			release,
			invocations: Arc::clone(&invocations),
		}))
		.unwrap();
	// SAFETY: Workers/tokens are independently retained and each is consumed
	// once.
	unsafe {
		let mut workers = Vec::new();
		for _ in 0..2 {
			let call = lk_bridge_submit(lk_app_plugins(&raw mut *app), id.0, 0, ptr::null(), 0);
			assert!(call > 0);
			let mut request = MaybeUninit::uninit();
			assert_eq!(lk_app_next_request(&raw mut *app, request.as_mut_ptr()), 1);
			let request = request.assume_init();
			let worker = lk_app_worker(&raw mut *app, &raw const request);
			lk_buffer_release(request.payload);
			workers.push(Box::from_raw(worker));
		}
		let second = workers.pop().unwrap();
		let token = lk_worker_cancellation(&raw const *second);
		let first = workers.pop().unwrap();
		let first_thread =
			std::thread::spawn(move || Box::from_raw(lk_worker_execute(Box::into_raw(first))));
		first_started
			.recv_timeout(std::time::Duration::from_secs(5))
			.unwrap();
		let (entered, entering) = std::sync::mpsc::channel();
		let second_thread = std::thread::spawn(move || {
			entered.send(()).unwrap();
			Box::from_raw(lk_worker_execute(Box::into_raw(second)))
		});
		entering
			.recv_timeout(std::time::Duration::from_secs(5))
			.unwrap();
		lk_worker_cancel(token);
		lk_worker_cancellation_release(token);
		drop(app);
		release_first.send(()).unwrap();
		lk_worker_reply_release(Box::into_raw(first_thread.join().unwrap()));
		lk_worker_reply_release(Box::into_raw(second_thread.join().unwrap()));
		assert_eq!(invocations.load(Ordering::SeqCst), 1);
	}
}
