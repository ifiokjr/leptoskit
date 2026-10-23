use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

use leptoskit_bridge::Error;
use leptoskit_bridge_ffi::app::App;
use leptoskit_bridge_ffi::app::AppService;
use leptoskit_bridge_ffi::app::ClosingApp;
use leptoskit_bridge_ffi::app::ServiceClose;
use leptoskit_bridge_ffi::app::ServiceWork;
use leptoskit_native::HeadlessBackend;
use leptoskit_wasm::WasmHost;

pub const GUEST: &[u8] = br#"(module
 (memory (export "memory") 1)
 (func (export "leptoskit_main"))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0)
 (func (export "leptoskit_plugins_ready"))
 (func (export "leptoskit_poll")))"#;

#[derive(Default)]
pub struct State {
	pub ready: AtomicBool,
	pub closed: AtomicUsize,
	pub joined: AtomicUsize,
	pub waker: Mutex<Option<Waker>>,
}

pub struct Service {
	pub state: Arc<State>,
	pub error: bool,
}

impl AppService for Service {
	fn poll(
		&mut self,
		host: &mut WasmHost<HeadlessBackend>,
		context: &mut Context<'_>,
	) -> Result<ServiceWork, Error> {
		*self.state.waker.lock().unwrap() = Some(context.waker().clone());
		let mut work = ServiceWork::default();

		while let Some(request) = host.plugins().next_request() {
			work.requests.push(request);
		}

		while let Some(call) = host.plugins().next_cancelled() {
			work.cancelled.push(call);
		}

		Ok(work)
	}

	fn close(self: Box<Self>) -> ServiceClose {
		self.state.closed.fetch_add(1, Ordering::AcqRel);
		let result = if self.error {
			Err(Error::Native)
		} else {
			Ok(())
		};

		Box::pin(std::future::poll_fn(move |context| {
			*self.state.waker.lock().unwrap() = Some(context.waker().clone());

			if !self.state.ready.load(Ordering::Acquire) {
				return Poll::Pending;
			}

			self.state.joined.fetch_add(1, Ordering::AcqRel);
			Poll::Ready(result)
		}))
	}
}

pub fn deferred(state: &Arc<State>, error: bool) -> App {
	let mut app = App::new(GUEST).unwrap();
	let state = Arc::clone(state);
	app.defer_service(Box::new(move |_| Ok(Box::new(Service { state, error }))))
		.unwrap();
	app
}

pub fn poll(owner: &mut ClosingApp) -> Poll<Result<(), Error>> {
	Pin::new(owner).poll(&mut Context::from_waker(Waker::noop()))
}
