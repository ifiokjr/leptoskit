//! The native guest's agent: reports what the hooks register through the
//! host's `DevTools` plugin, and applies the hub's commands for them.
//!
//! A guest has no executor of its own to borrow (each app brings one, or
//! none) and no timers, so the agent drives its own few calls. It polls them
//! with a waker that polls the agent again, which the bridge fires from
//! `leptoskit_plugins_ready` once the host answers one. Its pieces:
//!
//! - A hello, sent once, says what the guest can feed.
//! - One open `COMMANDS` call waits for the hub. Each reply's commands go
//!   through the registry, the same as on the web, and the app renders again;
//!   the call is then opened again.
//! - A flush, started when a hook marks a source dirty, waits [`DEBOUNCE_MS`]
//!   on the host's clock (the plugin's `WAIT`), so a burst of changes costs one
//!   report. It then collects the dirty sources through the same [`Outbox`] as
//!   the web agent and sends their values, a whole set ("reset") whenever the
//!   web agent would send a snapshot. One report posts at a time, so the host
//!   receives them in order.
//!
//! The wait is the host's rather than the display's: a display link stops
//! while the screen sleeps or the app is in the background, and asking for
//! frames would run the app's frame clock for every change, which `DevTools`
//! must not do.
//!
//! A hook marks a source dirty inside the signal write that changed it, so
//! [`Sink::wake`] only starts the wait. Values are read and commands applied
//! only when the agent is polled from the host.

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

use leptoskit_bridge::Error;
use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::NodeRef;
use leptoskit_devtools_protocol::Tree;
use leptoskit_plugin_devtools::Hello;
use leptoskit_plugin_devtools::Report;

use crate::registry;
use crate::registry::Sink;
use crate::report::Batch;
use crate::report::Outbox;

#[cfg(all(feature = "guest", target_arch = "wasm32"))]
mod bridge;
#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub(crate) use bridge::start;

/// How long a flush waits after the first change.
pub(crate) const DEBOUNCE_MS: u32 = 100;

/// How many reports in a row may fail before the agent waits for the next
/// change to try again.
const MAX_FAILURES: u32 = 3;

/// A call to the host.
pub(crate) type Pending<T> = Pin<Box<dyn Future<Output = Result<T, Error>>>>;

/// How the agent reaches the host: the bridge in a real guest, and a fake in
/// tests.
pub(crate) trait Transport: 'static {
	/// Says what the guest can feed.
	fn hello(&self, hello: &Hello) -> Pending<()>;
	/// Sends one payload from [`Report::encode`].
	fn report(&self, payload: &[u8]) -> Pending<()>;
	/// Waits for the hub's next commands for the guest.
	fn commands(&self) -> Pending<Vec<Command>>;
	/// Waits `ms` milliseconds on the host's clock.
	fn wait(&self, ms: u32) -> Pending<()>;
	/// Renders the app again, after a command changed its state.
	fn render(&self);
}

/// The guest's agent.
pub(crate) struct Agent<T> {
	transport: T,
	/// Polls the agent again when a call it waits on is answered.
	waker: Waker,
	outbox: RefCell<Outbox>,
	calls: RefCell<Calls>,
	/// Whether a flush is due: a source changed, or the hub needs every
	/// value again.
	changed: Cell<bool>,
	/// Whether the hub asked for every value. The outbox forgets what it
	/// sent when the next flush collects, so a report still posting can't
	/// undo it.
	resend: Cell<bool>,
	polling: Cell<bool>,
	again: Cell<bool>,
	failures: Cell<u32>,
}

#[derive(Default)]
struct Calls {
	hello: Option<Pending<()>>,
	commands: Option<Pending<Vec<Command>>>,
	flush: Flush,
}

#[derive(Default)]
enum Flush {
	#[default]
	Idle,
	/// Waiting out the debounce.
	Waiting(Pending<()>),
	/// The debounce is over.
	Due,
	/// Sending a batch's payloads, one at a time.
	Posting {
		batch: Batch,
		rest: VecDeque<Vec<u8>>,
		call: Pending<()>,
	},
}

impl<T: Transport> Agent<T> {
	pub(crate) fn new(transport: T, waker: Waker) -> Self {
		Self {
			transport,
			waker,
			outbox: RefCell::new(Outbox::default()),
			calls: RefCell::new(Calls::default()),
			changed: Cell::new(false),
			resend: Cell::new(false),
			polling: Cell::new(false),
			again: Cell::new(false),
			failures: Cell::new(0),
		}
	}

	/// Says hello, opens the call for commands, and schedules the first
	/// report, which carries every value.
	pub(crate) fn start(&self) {
		{
			let mut calls = self.calls.borrow_mut();
			calls.hello = Some(self.transport.hello(&Hello::new(registry::capabilities())));
			calls.commands = Some(self.transport.commands());
		}
		self.changed.set(true);
		self.poll();
	}

	/// Polls every call the agent waits on and acts on the answers.
	pub(crate) fn poll(&self) {
		if self.polling.replace(true) {
			self.again.set(true);
			return;
		}
		loop {
			self.again.set(false);
			self.step();
			if !self.again.get() {
				break;
			}
		}
		self.polling.set(false);
	}

	/// A source changed: start the debounce, without reading anything.
	fn schedule(&self) {
		self.changed.set(true);
		if self.polling.get() {
			self.again.set(true);
			return;
		}
		let mut calls = self.calls.borrow_mut();
		if matches!(calls.flush, Flush::Idle) {
			calls.flush = self.wait();
			let mut context = Context::from_waker(&self.waker);
			Self::arm(&mut calls, &mut context);
		}
	}

	fn step(&self) {
		let mut context = Context::from_waker(&self.waker);
		let commands = {
			let mut calls = self.calls.borrow_mut();
			if let Some(hello) = calls.hello.as_mut()
				&& hello.as_mut().poll(&mut context).is_ready()
			{
				calls.hello = None;
			}
			match calls
				.commands
				.as_mut()
				.map(|call| call.as_mut().poll(&mut context))
			{
				Some(Poll::Ready(Ok(commands))) => {
					calls.commands = Some(self.transport.commands());
					// The new call must be polled once to wait.
					self.again.set(true);
					commands
				}
				// The host closed, or refused: the hub's commands stop.
				Some(Poll::Ready(Err(_))) => {
					calls.commands = None;
					Vec::new()
				}
				Some(Poll::Pending) | None => Vec::new(),
			}
		};
		self.apply(commands);
		self.advance(&mut context);
	}

	/// Applies the hub's commands through the registry, as the web agent
	/// does, and renders what they changed.
	fn apply(&self, commands: Vec<Command>) {
		let mut render = false;
		for command in commands {
			if command == Command::RequestSnapshot {
				self.resend.set(true);
				self.changed.set(true);
				continue;
			}
			// A command for a value the app doesn't show has nothing to do;
			// the panels show what the guest reports, so nothing is lost.
			render |= registry::apply(&command).is_ok();
		}
		if render {
			self.transport.render();
		}
	}

	/// Moves the flush along as far as its calls allow.
	fn advance(&self, context: &mut Context<'_>) {
		loop {
			let mut calls = self.calls.borrow_mut();
			match &mut calls.flush {
				Flush::Idle => {
					if !self.changed.get() {
						return;
					}
					calls.flush = self.wait();
				}
				Flush::Waiting(_) => {
					if !Self::arm(&mut calls, context) {
						return;
					}
				}
				Flush::Due => {
					drop(calls);
					let flush = self.collect();
					self.calls.borrow_mut().flush = flush;
				}
				Flush::Posting { rest, call, .. } => {
					match call.as_mut().poll(context) {
						Poll::Pending => return,
						Poll::Ready(Ok(())) => {
							self.failures.set(0);
							if let Some(next) = rest.pop_front() {
								*call = self.transport.report(&next);
								continue;
							}
							if let Flush::Posting { batch, .. } = std::mem::take(&mut calls.flush) {
								self.outbox.borrow_mut().sent(batch);
							}
						}
						Poll::Ready(Err(_)) => {
							calls.flush = Flush::Idle;
							// The host may have only some of the values, so
							// the next report carries them all.
							self.outbox.borrow_mut().reset();
							let failures = self.failures.get() + 1;
							self.failures.set(failures);
							if failures < MAX_FAILURES {
								self.changed.set(true);
							}
						}
					}
				}
			}
		}
	}

	/// The debounce.
	fn wait(&self) -> Flush {
		Flush::Waiting(self.transport.wait(DEBOUNCE_MS))
	}

	/// Polls the debounce; true when it is over. A wait the host refuses
	/// ends it at once: the flush runs on this poll, or, when a hook started
	/// the wait from the app's path, on the next one the host's answers
	/// drive.
	fn arm(calls: &mut Calls, context: &mut Context<'_>) -> bool {
		let Flush::Waiting(wait) = &mut calls.flush else {
			return true;
		};
		if wait.as_mut().poll(context).is_pending() {
			return false;
		}
		calls.flush = Flush::Due;
		true
	}

	/// Collects the dirty sources and starts sending what changed.
	fn collect(&self) -> Flush {
		self.changed.set(false);
		if self.resend.replace(false) {
			self.outbox.borrow_mut().reset();
		}
		let Some(batch) = self.outbox.borrow().batch(false, empty_tree) else {
			return Flush::Idle;
		};
		let payloads = report(&batch).encode().unwrap_or_default();
		let mut rest = VecDeque::from(payloads);
		let Some(first) = rest.pop_front() else {
			self.outbox.borrow_mut().sent(batch);
			return Flush::Idle;
		};
		Flush::Posting {
			call: self.transport.report(&first),
			batch,
			rest,
		}
	}
}

/// The registry's [`Sink`]: a change starts a flush.
pub(crate) struct Wake<T>(pub(crate) Rc<Agent<T>>);

impl<T: Transport> Sink for Wake<T> {
	fn wake(&self) {
		self.0.schedule();
	}
}

/// The tree the outbox compares, which in a guest is always the same: the
/// host reports the tree.
fn empty_tree() -> Tree {
	Tree {
		root: NodeRef::Native(0),
		nodes: Vec::new(),
	}
}

/// The report for a batch: a snapshot becomes a reset carrying every value.
fn report(batch: &Batch) -> Report {
	let mut report = Report::default();
	for event in &batch.events {
		match event {
			Event::Snapshot { snapshot } => {
				report.reset = true;
				report.events.extend(registry::values(snapshot.clone()));
			}
			event => report.events.push(event.clone()),
		}
	}
	report
}

#[cfg(test)]
mod tests;
