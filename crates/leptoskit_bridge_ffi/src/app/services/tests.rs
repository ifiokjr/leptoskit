//! Public service ownership and cancellation controls use the actual
//! interpreter.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::CallId;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::PluginId;
use leptoskit_native::HeadlessBackend;
use leptoskit_wasm::WasmHost;

use crate::app::App;
use crate::app::AppService;
use crate::app::CloseScheduler;
use crate::app::ClosingApp;
use crate::app::ServiceClose;
use crate::app::ServiceWake;
use crate::app::ServiceWork;

const METHOD: MethodId = MethodId(7);
const VIEWPORT: (f32, f32) = (320.0, 480.0);
pub(super) const GUEST: &[u8] = br#"(module
 (import "leptoskit_plugins_v1" "submit" (func $submit (param i32 i32 i32 i32) (result i64)))
 (import "leptoskit_plugins_v1" "take_result" (func $take (param i64 i32 i32) (result i64)))
 (import "leptoskit" "commit" (func $commit))
 (import "leptoskit" "node_create" (func $create (param i64 i32)))
 (memory (export "memory") 1)
 (global $call (mut i64) (i64.const 0))
 (func (export "leptoskit_main")
  (call $create (i64.const 1) (i32.const 0))
  (global.set $call (call $submit (i32.const 0) (i32.const 7) (i32.const 0) (i32.const 0))))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0)
 (func (export "leptoskit_plugins_ready"))
 (func (export "leptoskit_poll")
  (if (i64.eq (call $take (global.get $call) (i32.const 0) (i32.const 2)) (i64.const 2))
   (then (call $commit))))
)"#;

#[derive(Default)]
pub(super) struct State {
	pub(super) ready: AtomicBool,
	pub(super) joined: AtomicBool,
	pub(super) closed: AtomicBool,
	pub(super) waker: Mutex<Option<Waker>>,
}

pub(super) struct Service {
	pub(super) state: Arc<State>,
	pub(super) call: Option<CallId>,
}

impl AppService for Service {
	fn poll(
		&mut self,
		host: &mut WasmHost<HeadlessBackend>,
		context: &mut Context<'_>,
	) -> Result<ServiceWork, Error> {
		let mut routed = ServiceWork::default();
		*self.state.waker.lock().unwrap() = Some(context.waker().clone());

		self.cancellations(host, &mut routed);
		self.requests(host, &mut routed);
		self.deliver(host)?;
		Ok(routed)
	}

	fn close(self: Box<Self>) -> ServiceClose {
		self.state.closed.store(true, Ordering::Release);
		Box::pin(std::future::poll_fn(move |context| {
			*self.state.waker.lock().unwrap() = Some(context.waker().clone());

			if !self.state.ready.load(Ordering::Acquire) {
				return Poll::Pending;
			}

			self.state.joined.store(true, Ordering::Release);
			Poll::Ready(Ok(()))
		}))
	}
}

impl Service {
	fn cancellations(&mut self, host: &mut WasmHost<HeadlessBackend>, routed: &mut ServiceWork) {
		while let Some(call) = host.plugins().next_cancelled() {
			if self.call == Some(call) {
				self.call = None;
				continue;
			}

			routed.cancelled.push(call);
		}
	}

	fn requests(&mut self, host: &mut WasmHost<HeadlessBackend>, routed: &mut ServiceWork) {
		while let Some(request) = host.plugins().next_request() {
			if request.plugin == PluginId(0) {
				self.call = Some(request.id);
				continue;
			}

			routed.requests.push(request);
		}
	}

	fn deliver(&mut self, host: &mut WasmHost<HeadlessBackend>) -> Result<(), Error> {
		if !self.state.ready.load(Ordering::Acquire) {
			return Ok(());
		}

		let Some(call) = self.call.take() else {
			return Ok(());
		};

		host.plugins().complete(call, Ok(Bytes::from(&b"ok"[..])))?;
		host.wake_plugins().unwrap();
		Ok(())
	}
}

struct Harness {
	app: App,
	state: Arc<State>,
	closing: Rc<RefCell<Vec<ClosingApp>>>,
	notifications: Arc<AtomicUsize>,
}

impl Harness {
	fn new() -> Self {
		let mut app = App::new(GUEST).unwrap();
		let state = Arc::new(State::default());
		let service_state = Arc::clone(&state);
		let closing = Rc::new(RefCell::new(Vec::new()));
		let queue = Rc::clone(&closing);
		let schedule: CloseScheduler = Rc::new(move |owner| queue.borrow_mut().push(owner));
		let notifications = Arc::new(AtomicUsize::new(0));
		let counter = Arc::clone(&notifications);

		app.defer_service(Box::new(move |host| {
			let id = host.plugins().register("service", [METHOD])?;
			host.plugins().grant(id, METHOD, true)?;
			Ok(Box::new(Service {
				state: service_state,
				call: None,
			}))
		}))
		.unwrap();
		assert!(
			app.with_host(|host| host.plugins().resolve("service"))
				.is_none()
		);
		app.activate_service(
			ServiceWake::new(move || {
				counter.fetch_add(1, Ordering::AcqRel);
			}),
			schedule,
		)
		.unwrap();
		Self {
			app,
			state,
			closing,
			notifications,
		}
	}
}

fn poll_close(owner: &mut ClosingApp) -> Poll<Result<(), Error>> {
	Pin::new(owner).poll(&mut Context::from_waker(Waker::noop()))
}

#[test]
fn real_guest_pending_wake_and_unrelated_requests_survive_owner_move() {
	let harness = Harness::new();
	let mut app = Box::new(harness.app);
	let identity = app.with_host(|host| (host.identity(), host.plugins().identity()));
	app.with_host(|host| host.start(VIEWPORT.0, VIEWPORT.1))
		.unwrap();
	app.poll_service().unwrap();
	assert_eq!(app.with_host(|host| host.commits()), 0);

	let unrelated = app.with_host(|host| {
		let id = host.plugins().register("unrelated", [METHOD]).unwrap();
		host.plugins().grant(id, METHOD, true).unwrap();
		host.plugins()
			.submit(id, METHOD, Bytes::from(&b"unrelated"[..]))
			.unwrap()
	});
	app.poll_service().unwrap();
	let request = app.next_request().unwrap();
	assert_eq!(request.id, unrelated);
	assert_eq!(&*request.payload, b"unrelated");

	app.with_host(|host| host.plugins().cancel(unrelated))
		.unwrap();
	app.poll_service().unwrap();
	assert_eq!(app.next_cancelled(), Some(unrelated));

	harness.state.ready.store(true, Ordering::Release);
	let waker = harness.state.waker.lock().unwrap().clone().unwrap();
	waker.wake_by_ref();
	waker.wake_by_ref();
	assert_eq!(harness.notifications.load(Ordering::Acquire), 1);

	app.poll_service().unwrap();
	assert_eq!(app.with_host(|host| host.commits()), 1);
	assert_eq!(
		identity,
		app.with_host(|host| (host.identity(), host.plugins().identity()))
	);

	drop(app);
	let mut close = harness.closing.borrow_mut().pop().unwrap();
	assert_eq!(poll_close(&mut close), Poll::Ready(Ok(())));
	assert!(harness.state.joined.load(Ordering::Acquire));
}

#[test]
fn pending_close_and_cancelled_await_retain_interpreter_payload_until_join() {
	let mut harness = Harness::new();
	let dropped = Arc::new(AtomicUsize::new(0));
	harness.app.with_host(|host| {
		let id = host.plugins().register("retained", [METHOD]).unwrap();
		host.plugins().grant(id, METHOD, true).unwrap();
		host.plugins()
			.submit(id, METHOD, Bytes::from_owner(Payload(Arc::clone(&dropped))))
			.unwrap();
	});

	let mut close = harness.app.close();
	assert!(harness.state.closed.load(Ordering::Acquire));
	assert_eq!(poll_close(&mut close), Poll::Pending);
	assert!(!harness.state.joined.load(Ordering::Acquire));
	assert_eq!(dropped.load(Ordering::Acquire), 0);

	drop(close);
	assert_eq!(harness.closing.borrow().len(), 1);

	harness.state.ready.store(true, Ordering::Release);
	let mut retained = harness.closing.borrow_mut().pop().unwrap();
	assert_eq!(poll_close(&mut retained), Poll::Ready(Ok(())));
	assert!(harness.state.joined.load(Ordering::Acquire));
	assert_eq!(dropped.load(Ordering::Acquire), 1);
	assert_eq!(poll_close(&mut retained), Poll::Ready(Ok(())));
}

struct Payload(Arc<AtomicUsize>);

impl AsRef<[u8]> for Payload {
	fn as_ref(&self) -> &[u8] {
		b"retained"
	}
}

impl Drop for Payload {
	fn drop(&mut self) {
		self.0.fetch_add(1, Ordering::AcqRel);
	}
}

#[test]
fn app_drop_closes_before_scheduling_and_late_concurrent_wakes_are_inert() {
	let mut harness = Harness::new();
	harness.app.poll_service().unwrap();
	let waker = harness.state.waker.lock().unwrap().clone().unwrap();

	drop(harness.app);
	assert!(harness.state.closed.load(Ordering::Acquire));
	assert_eq!(harness.closing.borrow().len(), 1);
	std::thread::scope(|scope| {
		for _ in 0..8 {
			let waker = waker.clone();
			scope.spawn(move || waker.wake());
		}
	});

	assert_eq!(harness.notifications.load(Ordering::Acquire), 0);
	harness.state.ready.store(true, Ordering::Release);
	let mut retained = harness.closing.borrow_mut().pop().unwrap();
	assert_eq!(poll_close(&mut retained), Poll::Ready(Ok(())));
}

struct FailedService(Service);

impl AppService for FailedService {
	fn poll(
		&mut self,
		_: &mut WasmHost<HeadlessBackend>,
		_: &mut Context<'_>,
	) -> Result<ServiceWork, Error> {
		Err(Error::Native)
	}

	fn close(self: Box<Self>) -> ServiceClose {
		let joined = Box::new(self.0).close();
		Box::pin(async move {
			joined.await?;
			Err(Error::Native)
		})
	}
}

#[test]
fn service_error_keeps_diagnostic_and_closes_after_join_with_stable_failure() {
	let state = Arc::new(State::default());
	let service_state = Arc::clone(&state);
	let mut app = App::new(GUEST).unwrap();
	app.defer_service(Box::new(move |_| {
		Ok(Box::new(FailedService(Service {
			state: service_state,
			call: None,
		})))
	}))
	.unwrap();
	app.activate_service(
		ServiceWake::new(|| {}),
		Rc::new(|_| panic!("explicit close is retained")),
	)
	.unwrap();
	assert_eq!(app.poll_service(), Err(Error::Native));
	assert!(app.error.contains("application service: Native"));

	let mut owner = app.close();
	assert_eq!(poll_close(&mut owner), Poll::Pending);
	assert!(!state.joined.load(Ordering::Acquire));
	state.ready.store(true, Ordering::Release);
	assert_eq!(poll_close(&mut owner), Poll::Ready(Err(Error::Native)));
	assert!(state.joined.load(Ordering::Acquire));
	assert_eq!(poll_close(&mut owner), Poll::Ready(Err(Error::Native)));
}

#[test]
fn cancellation_while_pending_prevents_late_delivery() {
	let mut harness = Harness::new();
	let call = harness
		.app
		.with_host(|host| host.plugins().submit(PluginId(0), METHOD, Bytes::default()))
		.unwrap();
	harness.app.poll_service().unwrap();
	let unrelated = harness.app.with_host(|host| {
		let plugin = host
			.plugins()
			.register("cancelled-native", [METHOD])
			.unwrap();
		host.plugins().grant(plugin, METHOD, true).unwrap();
		host.plugins()
			.submit(plugin, METHOD, Bytes::default())
			.unwrap()
	});
	harness.app.poll_service().unwrap();
	harness
		.app
		.with_host(|host| host.plugins().cancel(unrelated))
		.unwrap();

	assert!(
		harness
			.app
			.with_host(|host| host.plugins().result(call).unwrap().is_none())
	);
	harness
		.app
		.with_host(|host| host.plugins().cancel(call))
		.unwrap();
	harness.app.poll_service().unwrap();
	assert!(harness.app.next_request().is_none());
	assert_eq!(harness.app.next_cancelled(), Some(unrelated));
	harness.state.ready.store(true, Ordering::Release);
	harness.app.poll_service().unwrap();

	assert!(matches!(
		harness
			.app
			.with_host(|host| host.plugins().result(call).map(|_| ())),
		Err(Error::InvalidCall)
	));

	drop(harness.app);
	let mut owner = harness.closing.borrow_mut().pop().unwrap();
	assert_eq!(poll_close(&mut owner), Poll::Ready(Ok(())));
}
