//! A real channel-driven native task owned by the application's service.

use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;
use std::thread::JoinHandle;

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge_ffi::app::AppService;
use leptoskit_bridge_ffi::app::ServiceClose;
use leptoskit_bridge_ffi::app::ServiceInstaller;
use leptoskit_bridge_ffi::app::ServiceWork;
use leptoskit_native::HeadlessBackend;
use leptoskit_wasm::WasmHost;

pub static STARTED: AtomicUsize = AtomicUsize::new(0);
pub static JOINED: AtomicUsize = AtomicUsize::new(0);
pub static COMPLETED: AtomicUsize = AtomicUsize::new(0);
pub static FAIL_FACTORY: AtomicBool = AtomicBool::new(false);
static CLOSE_GATE: OnceLock<Mutex<Option<mpsc::Sender<()>>>> = OnceLock::new();

/// Releases the native worker after the close future has reported Pending.
pub fn release_close() {
	CLOSE_GATE
		.get()
		.unwrap()
		.lock()
		.unwrap()
		.take()
		.unwrap()
		.send(())
		.unwrap();
}

#[derive(Default)]
struct State {
	waker: Option<Waker>,
	stopped: bool,
}

struct Service {
	state: Arc<Mutex<State>>,
	commands: mpsc::Sender<bool>,
	worker: JoinHandle<()>,
	admitted: bool,
}

/// Produces only an inert installer; activation owns every running task.
pub fn from_app_dirs(data: &Path, cache: &Path) -> Result<ServiceInstaller, Error> {
	if FAIL_FACTORY.load(Ordering::SeqCst) || !data.is_dir() || !cache.is_dir() {
		return Err(Error::Native);
	}

	Ok(Box::new(|host| {
		host.plugins().register("fixture-session", [MethodId(1)])?;
		Ok(Box::new(start()))
	}))
}

fn start() -> Service {
	let state = Arc::new(Mutex::new(State::default()));
	let task_state = Arc::clone(&state);
	let (commands, receiver) = mpsc::channel();
	let (close, close_receiver) = mpsc::channel();
	*CLOSE_GATE.get_or_init(|| Mutex::new(None)).lock().unwrap() = Some(close);
	let worker = std::thread::spawn(move || run_task(&task_state, receiver, close_receiver));

	Service {
		state,
		commands,
		worker,
		admitted: false,
	}
}

fn notify(state: &mut State) {
	if let Some(waker) = state.waker.take() {
		waker.wake();
	}
}

fn run_task(state: &Mutex<State>, commands: mpsc::Receiver<bool>, close: mpsc::Receiver<()>) {
	STARTED.fetch_add(1, Ordering::SeqCst);

	while let Ok(run) = commands.recv() {
		if !run {
			break;
		}

		COMPLETED.fetch_add(1, Ordering::SeqCst);
		notify(&mut state.lock().unwrap());
	}

	close.recv().unwrap();
	let mut state = state.lock().unwrap();
	state.stopped = true;
	notify(&mut state);
}

impl AppService for Service {
	fn poll(
		&mut self,
		_: &mut WasmHost<HeadlessBackend>,
		context: &mut Context<'_>,
	) -> Result<ServiceWork, Error> {
		self.state.lock().unwrap().waker = Some(context.waker().clone());

		if !self.admitted {
			self.commands.send(true).map_err(|_| Error::Native)?;
			self.admitted = true;
		}

		Ok(ServiceWork::default())
	}

	fn close(self: Box<Self>) -> ServiceClose {
		let Self {
			state,
			commands,
			worker,
			..
		} = *self;
		let stopped = commands.send(false).is_ok();
		let mut worker = Some(worker);

		Box::pin(std::future::poll_fn(move |context| {
			poll_join(&state, &mut worker, stopped, context)
		}))
	}
}

fn poll_join(
	state: &Mutex<State>,
	worker: &mut Option<JoinHandle<()>>,
	stopped: bool,
	context: &mut Context<'_>,
) -> Poll<Result<(), Error>> {
	let mut state = state.lock().unwrap();

	if !state.stopped {
		state.waker = Some(context.waker().clone());

		return Poll::Pending;
	}

	drop(state);
	let result = worker.take().unwrap().join();
	JOINED.fetch_add(1, Ordering::SeqCst);

	Poll::Ready(if stopped && result.is_ok() {
		Ok(())
	} else {
		Err(Error::Native)
	})
}
