//! The guest thread: runs one application's guest, layout, and rendering off
//! the platform UI thread, so slow app logic never stops native scrolling,
//! animation, or touch handling. `docs/agents/guest-thread.md` has the design.
//!
//! The UI thread creates the [`App`], registers its plugins, and lends it to
//! [`lk_guest_spawn`]. Until [`lk_guest_stop`] returns, only the guest thread
//! touches it. Every other function here runs on the UI thread: each posts a
//! command or takes something the guest posted, and none waits for the guest.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::panic::AssertUnwindSafe;
use std::ptr;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::Duration;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::CallId;
use leptoskit_bridge::Error;
use leptoskit_bridge::frame::Frame;
use leptoskit_native::NativeEvent;
use leptoskit_native::RiveEnvironment;
use leptoskit_native::RiveError;
use leptoskit_native::RiveFrame;
use leptoskit_native::RivePresenter;
use leptoskit_yoga::NodeId;

use super::App;
use super::AsyncJob;
use super::NextRequest;
use super::RustWorker;
use super::WorkerReply;
use super::binary;
use super::render::MAX_TRANSACTION_BYTES;
use crate::Buffer;
use crate::NativeRequest;

/// Native requests the guest moves ahead of the UI thread. The platform's job
/// budget bounds the calls in flight; this bounds the ones waiting for it.
const MAX_WAITING_REQUESTS: usize = 64;

/// How the guest thread reaches the UI thread.
#[repr(C)]
pub struct GuestCallbacks {
	/// Passed to both callbacks.
	pub context: *mut c_void,
	/// Runs on the guest thread when it has posted something: a transaction,
	/// a request, a cancellation, a Rive frame, a changed frame or Rive flag,
	/// or a failure. Post a turn to the UI thread, and there call
	/// [`lk_guest_ack`] before taking what the guest posted. It runs again
	/// only after that acknowledgement.
	pub notify: Option<unsafe extern "C" fn(*mut c_void)>,
	/// Runs once, on the thread calling [`lk_guest_stop`], after the guest
	/// thread has exited, so no `notify` can follow.
	pub release: Option<unsafe extern "C" fn(*mut c_void)>,
}

/// The guest thread of one application.
pub struct Guest {
	shared: Arc<Shared>,
	thread: JoinHandle<()>,
}

/// What the UI thread and the guest thread share.
struct Shared {
	state: Mutex<State>,
	/// Wakes the guest thread when the inbox gains something.
	work: Condvar,
	notifier: Notifier,
	/// Set by a notification until the UI thread acknowledges it.
	notified: AtomicBool,
	/// Whether the guest waits for a display frame.
	frame_wanted: AtomicBool,
	/// Whether the guest owns Rive scenes, including paused ones.
	has_rive: AtomicBool,
	/// Whether the last Rive tick needs another.
	rive_pending: AtomicBool,
	failed: AtomicBool,
}

#[derive(Default)]
struct State {
	// The inbox, from the UI thread.
	commands: VecDeque<Command>,
	/// The newest display frame not yet delivered: a slow guest skips to it
	/// rather than working through every frame it missed.
	frame: Option<Frame>,
	/// The UI thread took a request after the guest filled the outbox.
	refill: bool,
	stop: bool,
	// The outbox, from the guest thread.
	transactions: VecDeque<Bytes>,
	requests: VecDeque<PlatformRequest>,
	/// Whether the guest left native requests behind for want of room.
	requests_capped: bool,
	cancelled: VecDeque<u64>,
	rive_images: Vec<RiveImage>,
	error: Option<String>,
}

enum Command {
	Start {
		width: f32,
		height: f32,
	},
	Resize {
		width: f32,
		height: f32,
		dark: bool,
	},
	Event {
		node: usize,
		event: NativeEvent,
	},
	Complete {
		id: CallId,
		reply: Result<Bytes, Error>,
	},
	CompleteWorker {
		id: CallId,
		reply: Box<WorkerReply>,
	},
	/// A service or async plugin has work: run a step for it.
	Wake,
	RiveTick {
		elapsed: Duration,
		environment: RiveEnvironment,
		visible: Vec<usize>,
	},
}

/// A call for the platform, with the Rust job it resolved to, if any.
struct PlatformRequest {
	request: NativeRequest,
	worker: *mut RustWorker,
	job: *mut AsyncJob,
}

// SAFETY: The payload buffer, the worker, and the async job each own their
// allocation and refer to no app, bridge, or interpreter; the FFI already
// hands them to other threads to run.
unsafe impl Send for PlatformRequest {}

impl PlatformRequest {
	/// Hands every part to the caller, who releases each once.
	fn into_parts(self) -> (NativeRequest, *mut RustWorker, *mut AsyncJob) {
		let parts = ManuallyDrop::new(self);
		// SAFETY: `parts` is never dropped, so the request moves out once.
		(
			unsafe { ptr::read(&raw const parts.request) },
			parts.worker,
			parts.job,
		)
	}
}

impl Drop for PlatformRequest {
	/// Releases a request the UI thread never took.
	fn drop(&mut self) {
		// SAFETY: An untaken request still owns all three allocations.
		unsafe {
			crate::lk_buffer_release(ptr::read(&raw const self.request.payload));
			super::lk_worker_release(self.worker);
			super::lk_async_release(self.job);
		}
	}
}

/// One rendered Rive frame, copied for the UI thread to present.
struct RiveImage {
	node: usize,
	width: u32,
	height: u32,
	pixels: Vec<u8>,
}

/// The platform's callbacks, as plain values the threads can share.
struct Notifier {
	context: usize,
	notify: unsafe extern "C" fn(*mut c_void),
	release: unsafe extern "C" fn(*mut c_void),
}

/// The app, lent to the guest thread.
struct LentApp(*mut App);

// SAFETY: `lk_guest_spawn`'s caller lends the app exclusively until
// `lk_guest_stop` joins the thread, and nothing inside it is shared with code
// outside it. Spawning and joining the thread order every access, so the app
// is only ever used by one thread at a time. Its Rive players, which must
// stay on one thread, are created, advanced, and released on the guest.
unsafe impl Send for LentApp {}

impl Shared {
	fn new(notifier: Notifier) -> Self {
		Self {
			state: Mutex::default(),
			work: Condvar::new(),
			notifier,
			notified: AtomicBool::new(false),
			frame_wanted: AtomicBool::new(false),
			has_rive: AtomicBool::new(false),
			rive_pending: AtomicBool::new(false),
			failed: AtomicBool::new(false),
		}
	}

	/// The state lock. Every holder only moves values in or out, so a panic
	/// while it is held cannot leave the queues half changed.
	fn lock(&self) -> MutexGuard<'_, State> {
		self.state.lock().unwrap_or_else(PoisonError::into_inner)
	}

	/// Queues a command and wakes the guest thread.
	fn post(&self, command: Command) {
		let mut state = self.lock();

		if state.stop {
			return;
		}

		match (&command, state.commands.back_mut()) {
			// The guest only needs where the view settled.
			(
				Command::Event {
					node,
					event: NativeEvent::Scroll { .. },
				},
				Some(Command::Event {
					node: queued,
					event: queued_event @ NativeEvent::Scroll { .. },
				}),
			) if node == queued => {
				if let Command::Event { event, .. } = command {
					*queued_event = event;
				}
			}
			(Command::Wake, Some(Command::Wake)) => {}
			_ => state.commands.push_back(command),
		}

		drop(state);
		self.work.notify_one();
	}

	fn post_frame(&self, frame: Frame) {
		let mut state = self.lock();

		if state.stop {
			return;
		}

		state.frame = Some(frame);
		drop(state);
		self.work.notify_one();
	}

	/// The commands and frame to run next, or `None` once asked to stop.
	fn next_batch(&self) -> Option<(VecDeque<Command>, Option<Frame>)> {
		let mut state = self.lock();

		while !state.stop && state.commands.is_empty() && state.frame.is_none() && !state.refill {
			state = self
				.work
				.wait(state)
				.unwrap_or_else(PoisonError::into_inner);
		}

		if state.stop {
			return None;
		}

		state.refill = false;
		Some((std::mem::take(&mut state.commands), state.frame.take()))
	}

	/// Tells the UI thread to look, unless it has not looked since the last
	/// time.
	fn notify(&self) {
		if !self.notified.swap(true, Ordering::AcqRel) {
			// SAFETY: The platform keeps the context alive until `release`,
			// which runs only after this thread has exited.
			unsafe { (self.notifier.notify)(self.notifier.context as *mut c_void) };
		}
	}

	/// Records the first failure. The guest takes no more commands.
	fn fail(&self, message: String) {
		self.lock().error.get_or_insert(message);
		self.failed.store(true, Ordering::Release);
		self.notify();
	}
}

/// Lends an app to a new guest thread. Returns null, leaving the app and the
/// callbacks' context with the caller, when either callback is null or the
/// thread cannot start.
///
/// Create the app, register its plugins, set its text measurer, enable its
/// frames, and activate its service first: from here until
/// [`lk_guest_stop`] returns, do not pass the app to any other function.
///
/// # Safety
/// `app` is a live app the caller owns and stops touching. `context` stays
/// valid for both callbacks until `release` runs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_spawn(app: *mut App, callbacks: GuestCallbacks) -> *mut Guest {
	let (Some(notify), Some(release)) = (callbacks.notify, callbacks.release) else {
		return ptr::null_mut();
	};

	if app.is_null() {
		return ptr::null_mut();
	}

	let shared = Arc::new(Shared::new(Notifier {
		context: callbacks.context as usize,
		notify,
		release,
	}));
	let lent = LentApp(app);
	let thread = std::thread::Builder::new()
		.name("leptoskit-guest".into())
		.spawn({
			let shared = Arc::clone(&shared);
			move || lent.run(&shared)
		});

	match thread {
		Ok(thread) => Box::into_raw(Box::new(Guest { shared, thread })),
		Err(_) => ptr::null_mut(),
	}
}

/// Stops the guest thread and gives the app back. Waits for the step in
/// progress, then for the guest to release its Rive players on its own
/// thread, then runs the `release` callback. Commands still queued and
/// requests never taken are dropped. Returns 0, or -1 when the guest thread
/// panicked outside a step; either way the app is the caller's again, to
/// close.
///
/// # Safety
/// `guest` is null or a live guest from [`lk_guest_spawn`], stopped once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_stop(guest: *mut Guest) -> i32 {
	if guest.is_null() {
		return -1;
	}

	// SAFETY: The caller returns the allocation from `lk_guest_spawn` once.
	let guest = unsafe { Box::from_raw(guest) };
	guest.shared.lock().stop = true;
	guest.shared.work.notify_one();
	let joined = guest.thread.join();
	let notifier = &guest.shared.notifier;
	// SAFETY: The guest thread has exited, so no notification can follow.
	unsafe { (notifier.release)(notifier.context as *mut c_void) };

	if joined.is_ok() { 0 } else { -1 }
}

/// Acknowledges a notification. Call it on the UI thread before taking what
/// the guest posted, so anything posted afterwards notifies again.
///
/// # Safety
/// `guest` is null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_ack(guest: *const Guest) {
	// SAFETY: The caller lends a live guest.
	if let Some(guest) = unsafe { guest.as_ref() } {
		guest.shared.notified.store(false, Ordering::Release);
	}
}

/// Starts the guest at a viewport size, in layout units.
///
/// # Safety
/// `guest` is null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_start(guest: *const Guest, width: f32, height: f32) -> i32 {
	// SAFETY: The caller lends a live guest.
	unsafe { post(guest, Command::Start { width, height }) }
}

/// Lays the screen out again for a new viewport or appearance.
///
/// # Safety
/// `guest` is null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_resize(
	guest: *const Guest,
	width: f32,
	height: f32,
	dark: bool,
) -> i32 {
	// SAFETY: The caller lends a live guest.
	unsafe {
		post(
			guest,
			Command::Resize {
				width,
				height,
				dark,
			},
		)
	}
}

/// Sends an event, as [`super::lk_app_event`] describes it, for the guest to
/// dispatch. A scroll replaces the previous scroll of the same view while
/// the guest has not taken it. A malformed event fails the guest.
///
/// # Safety
/// `guest` is null or live; `text` addresses `len` bytes, null only when 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_event(
	guest: *const Guest,
	node: usize,
	kind: u32,
	text: *const u8,
	len: usize,
) -> i32 {
	// SAFETY: The caller lends a live guest and the event's text.
	let (Some(guest), event) = (unsafe { guest.as_ref() }, unsafe {
		super::decode_event(kind, text, len)
	}) else {
		return -1;
	};

	match event {
		Ok(event) => {
			guest.shared.post(Command::Event { node, event });
			0
		}
		Err(error) => {
			guest.shared.fail(error.into());
			-1
		}
	}
}

/// Delivers a display frame from the platform's display link. A frame the
/// guest has not taken yet is replaced, so a slow guest skips to the newest.
///
/// # Safety
/// `guest` is null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_frame(
	guest: *const Guest,
	timestamp_ns: u64,
	period_ns: u64,
) -> i32 {
	// SAFETY: The caller lends a live guest.
	let Some(guest) = (unsafe { guest.as_ref() }) else {
		return -1;
	};
	guest.shared.post_frame(Frame {
		timestamp_ns,
		period_ns,
	});
	0
}

/// Whether the guest waits for a display frame (1) or not (0). Run the
/// display link only while this is 1; a notification follows each change.
///
/// # Safety
/// `guest` is null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_frame_requested(guest: *const Guest) -> i32 {
	// SAFETY: The caller lends a live guest.
	unsafe { flag(guest, |shared| &shared.frame_wanted) }
}

/// Runs a guest step for a service or async plugin that has work.
///
/// # Safety
/// `guest` is null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_wake(guest: *const Guest) -> i32 {
	// SAFETY: The caller lends a live guest.
	unsafe { post(guest, Command::Wake) }
}

/// Completes a platform plugin call, as `lk_bridge_complete` does: `error` 0
/// with the reply's bytes, which are copied, or a plugin error code.
///
/// # Safety
/// `guest` is null or live; `data` addresses `len` bytes, null only when 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_complete(
	guest: *const Guest,
	id: u64,
	error: i32,
	data: *const u8,
	len: usize,
) -> i32 {
	let reply = if error != 0 {
		Err(Error::from_status(-i64::from(error) - 1))
	} else if len == 0 {
		Ok(Bytes::new())
	} else if data.is_null() {
		Err(Error::InvalidArgument)
	} else {
		// SAFETY: The caller lends `len` readable bytes, copied here.
		Ok(Bytes::copy_from_slice(unsafe {
			std::slice::from_raw_parts(data, len)
		}))
	};
	// SAFETY: The caller lends a live guest.
	unsafe {
		post(
			guest,
			Command::Complete {
				id: CallId(id),
				reply,
			},
		)
	}
}

/// Completes a call with a worker's or async job's reply, moving its bytes.
/// The reply is consumed on every path.
///
/// # Safety
/// `guest` is null or live; `reply` uniquely owns an unreleased reply.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_complete_worker(
	guest: *const Guest,
	id: u64,
	reply: *mut WorkerReply,
) -> i32 {
	if reply.is_null() {
		return -1;
	}

	// SAFETY: The caller relinquishes the reply on every path.
	let reply = unsafe { Box::from_raw(reply) };
	// SAFETY: The caller lends a live guest.
	unsafe {
		post(
			guest,
			Command::CompleteWorker {
				id: CallId(id),
				reply,
			},
		)
	}
}

/// Takes the oldest transaction, in the binary form `lk_app_render_binary`
/// sends. Apply them in order. An empty buffer means none is waiting.
///
/// # Safety
/// `guest` is null or live. Release the returned buffer once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_take_render(guest: *const Guest) -> Buffer {
	// SAFETY: The caller lends a live guest.
	let transaction =
		unsafe { guest.as_ref() }.and_then(|guest| guest.shared.lock().transactions.pop_front());
	Buffer::new(transaction.unwrap_or_default())
}

/// Takes the next platform plugin call: 1 when one was written to `out`, 0
/// when none is waiting. A call for a worker or async Rust plugin comes with
/// its job in `worker` or `job`, otherwise null; run or release it as
/// `lk_app_worker` and `lk_app_async` describe, and release the request's
/// payload as `lk_app_next_request` does.
///
/// # Safety
/// `guest` is null or live; the outputs are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_next_request(
	guest: *const Guest,
	out: *mut NativeRequest,
	worker: *mut *mut RustWorker,
	job: *mut *mut AsyncJob,
) -> i32 {
	// SAFETY: The caller lends a live guest.
	let Some(guest) = (unsafe { guest.as_ref() }) else {
		return -1;
	};

	if out.is_null() || worker.is_null() || job.is_null() {
		return -1;
	}

	let mut state = guest.shared.lock();
	let Some(request) = state.requests.pop_front() else {
		return 0;
	};
	let refill = std::mem::take(&mut state.requests_capped);
	state.refill |= refill;
	drop(state);

	if refill {
		guest.shared.work.notify_one();
	}

	let (request, taken_worker, taken_job) = request.into_parts();
	// SAFETY: The caller lends writable storage for each output.
	unsafe {
		out.write(request);
		worker.write(taken_worker);
		job.write(taken_job);
	}
	1
}

/// Takes the next cancelled call's ID, or 0 when none is waiting.
///
/// # Safety
/// `guest` is null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_next_cancelled(guest: *const Guest) -> i64 {
	// SAFETY: The caller lends a live guest.
	unsafe { guest.as_ref() }
		.and_then(|guest| guest.shared.lock().cancelled.pop_front())
		.map_or(0, |id| i64::try_from(id).unwrap_or(-1))
}

/// Whether the guest owns Rive scenes, including paused ones (1) or not (0).
///
/// # Safety
/// `guest` is null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_has_rive(guest: *const Guest) -> i32 {
	// SAFETY: The caller lends a live guest.
	unsafe { flag(guest, |shared| &shared.has_rive) }
}

/// Asks the guest to advance its Rive scenes. `visible` lists the Rive views
/// the UI thread can see; frames come back through
/// [`lk_guest_take_rive_frame`]. Returns whether the last tick the guest ran
/// wanted another (1) or not (0), or -1 for invalid input. Flags: bit 0 =
/// active visible window, bit 1 = reduced motion.
///
/// # Safety
/// `guest` is null or live; `visible` addresses `count` IDs, null only
/// when 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_rive_tick(
	guest: *const Guest,
	elapsed: f64,
	scale: f32,
	flags: u32,
	visible: *const usize,
	count: usize,
) -> i32 {
	// SAFETY: The caller lends a live guest.
	let Some(guest) = (unsafe { guest.as_ref() }) else {
		return -1;
	};

	if !elapsed.is_finite()
		|| elapsed < 0.0
		|| !scale.is_finite()
		|| scale <= 0.0
		|| flags & !3 != 0
		|| (visible.is_null() && count != 0)
	{
		return -1;
	}

	let visible = if count == 0 {
		Vec::new()
	} else {
		// SAFETY: The caller lends `count` readable IDs.
		unsafe { std::slice::from_raw_parts(visible, count) }.to_vec()
	};
	guest.shared.post(Command::RiveTick {
		elapsed: Duration::from_secs_f64(elapsed.min(0.1)),
		environment: RiveEnvironment {
			scale,
			active: flags & 1 != 0,
			reduced_motion: flags & 2 != 0,
		},
		visible,
	});
	i32::from(guest.shared.rive_pending.load(Ordering::Acquire))
}

/// Takes the next rendered Rive frame: its RGBA pixels, premultiplied, with
/// the view and size written to the outputs. An empty buffer means none is
/// waiting. Only the newest frame of each view is kept.
///
/// # Safety
/// `guest` is null or live; the outputs are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_take_rive_frame(
	guest: *const Guest,
	node: *mut usize,
	width: *mut u32,
	height: *mut u32,
) -> Buffer {
	// SAFETY: The caller lends a live guest.
	let image = unsafe { guest.as_ref() }.and_then(|guest| guest.shared.lock().rive_images.pop());

	let Some(image) = image.filter(|_| !node.is_null() && !width.is_null() && !height.is_null())
	else {
		return Buffer::new(Bytes::new());
	};
	// SAFETY: The caller lends writable storage for each output.
	unsafe {
		node.write(image.node);
		width.write(image.width);
		height.write(image.height);
	}
	Buffer::new(image.pixels.into())
}

/// Whether the guest has failed (1) or not (0). Read why with
/// [`lk_guest_error`], then stop it.
///
/// # Safety
/// `guest` is null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_failed(guest: *const Guest) -> i32 {
	// SAFETY: The caller lends a live guest.
	unsafe { flag(guest, |shared| &shared.failed) }
}

/// Copies why the guest failed, or nothing when it has not.
///
/// # Safety
/// `guest` is null or live. Release the returned buffer once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_guest_error(guest: *const Guest) -> Buffer {
	// SAFETY: The caller lends a live guest.
	let message = unsafe { guest.as_ref() }.map_or_else(
		|| "invalid guest".to_owned(),
		|guest| guest.shared.lock().error.clone().unwrap_or_default(),
	);
	Buffer::new(message.into())
}

/// # Safety
/// `guest` is null or live.
unsafe fn post(guest: *const Guest, command: Command) -> i32 {
	// SAFETY: The caller lends a live guest.
	match unsafe { guest.as_ref() } {
		Some(guest) => {
			guest.shared.post(command);
			0
		}
		None => -1,
	}
}

/// # Safety
/// `guest` is null or live.
unsafe fn flag(guest: *const Guest, flag: impl FnOnce(&Shared) -> &AtomicBool) -> i32 {
	// SAFETY: The caller lends a live guest.
	unsafe { guest.as_ref() }.map_or(-1, |guest| {
		i32::from(flag(&guest.shared).load(Ordering::Acquire))
	})
}

impl LentApp {
	/// The guest thread's loop: wait for work, run it, post what came of it.
	fn run(self, shared: &Shared) {
		raise_priority();
		// SAFETY: See `LentApp`: the app is this thread's alone until it exits.
		run(unsafe { &mut *self.0 }, shared);
	}
}

fn run(app: &mut App, shared: &Shared) {
	while let Some((commands, frame)) = shared.next_batch() {
		// A failed guest drops what it is sent until it is stopped.
		if shared.failed.load(Ordering::Acquire) {
			continue;
		}

		let outcome =
			std::panic::catch_unwind(AssertUnwindSafe(|| step(app, shared, commands, frame)));

		match outcome {
			Ok(Ok(())) => {}
			Ok(Err(message)) => shared.fail(message),
			Err(_) => shared.fail("the guest thread panicked".into()),
		}
	}

	// Rive players must be dropped on the thread that created them.
	if let Some(host) = app.host.as_mut() {
		let _ = std::panic::catch_unwind(AssertUnwindSafe(|| host.release_rive_players()));
	}
}

/// Runs one batch of commands and the frame, then does what a UI-thread pump
/// does: poll the service, move cancellations and native requests to the
/// outbox, and render.
fn step(
	app: &mut App,
	shared: &Shared,
	commands: VecDeque<Command>,
	frame: Option<Frame>,
) -> Result<(), String> {
	let mut rive_images = Vec::new();
	let mut rive_pending = None;

	for command in commands {
		let status = match command {
			Command::Start { width, height } => app.start(width, height),
			Command::Resize {
				width,
				height,
				dark,
			} => app.resize(width, height, dark),
			Command::Event { node, event } => app.dispatch_event(node, &event),
			Command::Complete { id, reply } => complete(app, id, reply),
			Command::CompleteWorker { id, reply } => complete_worker(app, id, *reply),
			Command::Wake => 0,
			Command::RiveTick {
				elapsed,
				environment,
				visible,
			} => {
				let mut presenter = Snapshot {
					visible: &visible,
					images: &mut rive_images,
				};
				match app
					.host_mut()
					.tick_rive_with(elapsed, environment, &mut presenter)
				{
					Ok(pending) => {
						rive_pending = Some(pending);
						0
					}
					Err(error) => {
						app.error = error.to_string();
						-1
					}
				}
			}
		};

		if status < 0 {
			return Err(app.error.clone());
		}
	}

	if let Some(frame) = frame
		&& app.deliver_frame(frame) < 0
	{
		return Err(app.error.clone());
	}

	if app.poll_service().is_err() {
		return Err(app.error.clone());
	}

	let mut cancelled = Vec::new();

	while let Some(id) = app.next_cancelled() {
		cancelled.push(id.0);
	}

	let room = MAX_WAITING_REQUESTS.saturating_sub(shared.lock().requests.len());
	let mut requests = Vec::new();
	let mut capped = false;
	let mut more = false;

	loop {
		if requests.len() == room {
			capped = true;
			break;
		}

		match app.next_platform_request() {
			Err(()) => return Err(app.error.clone()),
			Ok(NextRequest::Idle) => break,
			// Render what the answered Rust calls changed, then run again.
			Ok(NextRequest::Yield) => {
				more = true;
				break;
			}
			Ok(NextRequest::Platform(request)) => requests.push(platform_request(app, request)),
		}
	}

	let transaction = if app.render_pending() {
		let bytes = app
			.encode_render(|root, mutations| {
				binary::encode(root, mutations, MAX_TRANSACTION_BYTES)
					.map_err(|error| error.to_string())
			})
			.ok_or_else(|| app.error.clone())?;
		Some(Bytes::from(bytes))
	} else {
		None
	};

	let frame_wanted = app.wants_frame();
	let has_rive = app
		.host
		.as_ref()
		.is_some_and(leptoskit_wasm::WasmHost::has_rive_players);
	let flags_changed = shared.frame_wanted.swap(frame_wanted, Ordering::AcqRel)
		!= frame_wanted
			| (shared.has_rive.swap(has_rive, Ordering::AcqRel) != has_rive)
			| rive_pending.is_some_and(|pending| {
				shared.rive_pending.swap(pending, Ordering::AcqRel) != pending
			});
	let posted = transaction.is_some()
		|| !requests.is_empty()
		|| !cancelled.is_empty()
		|| !rive_images.is_empty();

	let mut state = shared.lock();
	state.transactions.extend(transaction);
	state.requests.extend(requests);
	state.requests_capped |= capped;
	state.cancelled.extend(cancelled);

	for image in rive_images {
		state.rive_images.retain(|queued| queued.node != image.node);
		state.rive_images.push(image);
	}

	state.refill |= more;
	drop(state);

	if posted || flags_changed {
		shared.notify();
	}

	Ok(())
}

/// Completes a platform plugin call. A call the guest stopped waiting for is
/// ignored, as the UI-thread runtimes ignore it.
fn complete(app: &mut App, id: CallId, reply: Result<Bytes, Error>) -> i32 {
	let bridge = app.host_mut().plugins();
	let reply = reply.and_then(|bytes| bridge.check_reply(id, bytes.len()).map(|()| bytes));
	let completed = bridge.complete(id, reply);
	woken(app, completed)
}

fn complete_worker(app: &mut App, id: CallId, reply: WorkerReply) -> i32 {
	let completed = app.host_mut().plugins().complete_reply(id, reply.0);
	woken(app, completed)
}

/// Lets the guest run after a completion.
fn woken(app: &mut App, completed: Result<(), Error>) -> i32 {
	match completed {
		Ok(()) => {
			let result = app.host_mut().wake_plugins();
			app.record(result)
		}
		Err(Error::InvalidCall) => 0,
		Err(error) => {
			app.error = format!("complete a platform plugin call: {error:?}");
			-1
		}
	}
}

/// Resolves a call for a worker or async Rust plugin into its job here, on
/// the app's thread, so the UI thread can run it without the app.
fn platform_request(app: &mut App, request: leptoskit_bridge::Request) -> PlatformRequest {
	let request = NativeRequest::new(request);
	// SAFETY: The app is this thread's, and the request's payload is live.
	let worker = unsafe { super::lk_app_worker(app, &raw const request) };
	let job = if worker.is_null() {
		// SAFETY: As above.
		unsafe { super::lk_app_async(app, &raw const request) }
	} else {
		ptr::null_mut()
	};

	PlatformRequest {
		request,
		worker,
		job,
	}
}

/// Presents Rive frames into owned copies, for the UI thread to show, and
/// answers visibility from the UI thread's last look.
struct Snapshot<'a> {
	visible: &'a [usize],
	images: &'a mut Vec<RiveImage>,
}

impl RivePresenter for Snapshot<'_> {
	fn visible(&mut self, id: NodeId) -> bool {
		self.visible.contains(&id.index())
	}

	fn present(&mut self, id: NodeId, frame: &RiveFrame<'_>) -> Result<(), RiveError> {
		self.images.retain(|image| image.node != id.index());
		self.images.push(RiveImage {
			node: id.index(),
			width: frame.width(),
			height: frame.height(),
			pixels: frame.bytes().to_vec(),
		});
		Ok(())
	}
}

/// Gives the guest thread the scheduling of threads that draw: it builds
/// every frame the UI thread shows. The request is a hint the system may
/// refuse; the guest works the same at any priority.
fn raise_priority() {
	#[cfg(any(target_os = "ios", target_os = "macos"))]
	{
		unsafe extern "C" {
			fn pthread_set_qos_class_self_np(class: u32, relative_priority: i32) -> i32;
		}
		/// `QOS_CLASS_USER_INTERACTIVE`.
		const USER_INTERACTIVE: u32 = 0x21;
		// SAFETY: Changes only the calling thread's quality of service.
		unsafe { pthread_set_qos_class_self_np(USER_INTERACTIVE, 0) };
	}

	#[cfg(target_os = "android")]
	{
		unsafe extern "C" {
			fn setpriority(which: i32, who: u32, priority: i32) -> i32;
		}
		/// `PRIO_PROCESS`: with `who` 0, the calling thread on Linux.
		const PRIO_PROCESS: i32 = 0;
		/// `THREAD_PRIORITY_DISPLAY`.
		const DISPLAY: i32 = -4;
		// SAFETY: Changes only the calling thread's nice value.
		unsafe { setpriority(PRIO_PROCESS, 0, DISPLAY) };
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	unsafe extern "C" fn ignore(_: *mut c_void) {}

	fn shared() -> Shared {
		Shared::new(Notifier {
			context: 0,
			notify: ignore,
			release: ignore,
		})
	}

	fn scroll(node: usize, offset_y: f32) -> Command {
		Command::Event {
			node,
			event: NativeEvent::Scroll {
				offset_x: 0.0,
				offset_y,
			},
		}
	}

	fn describe(command: &Command) -> String {
		match command {
			Command::Event {
				node,
				event: NativeEvent::Scroll { offset_y, .. },
			} => format!("scroll {node} {offset_y}"),
			Command::Event { node, .. } => format!("event {node}"),
			Command::Wake => "wake".into(),
			_ => "other".into(),
		}
	}

	#[test]
	fn a_scroll_replaces_the_queued_scroll_of_its_view_only() {
		let shared = shared();
		shared.post(scroll(1, 10.0));
		shared.post(scroll(1, 20.0));
		shared.post(scroll(2, 5.0));
		shared.post(scroll(1, 30.0));
		shared.post(Command::Event {
			node: 1,
			event: NativeEvent::Press,
		});
		shared.post(scroll(1, 40.0));
		shared.post(Command::Wake);
		shared.post(Command::Wake);

		let (commands, _) = shared.next_batch().unwrap();
		let commands: Vec<String> = commands.iter().map(describe).collect();
		assert_eq!(
			commands,
			[
				"scroll 1 20",
				"scroll 2 5",
				"scroll 1 30",
				"event 1",
				"scroll 1 40",
				"wake"
			]
		);
	}

	#[test]
	fn the_newest_frame_replaces_one_not_yet_delivered() {
		let shared = shared();

		for timestamp_ns in [1, 2, 3] {
			shared.post_frame(Frame {
				timestamp_ns,
				period_ns: 16,
			});
		}

		let (commands, frame) = shared.next_batch().unwrap();
		assert!(commands.is_empty());
		assert_eq!(frame.map(|frame| frame.timestamp_ns), Some(3));
	}

	#[test]
	fn a_stopped_guest_takes_nothing_more() {
		let shared = shared();
		shared.lock().stop = true;
		shared.post(Command::Wake);
		shared.post_frame(Frame {
			timestamp_ns: 1,
			period_ns: 16,
		});

		assert!(shared.next_batch().is_none());
		assert!(shared.lock().commands.is_empty());
	}
}
