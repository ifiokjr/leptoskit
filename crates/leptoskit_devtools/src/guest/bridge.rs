//! The agent over the real bridge, in a guest the host runs.

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::task::Waker;

use leptoskit_devtools_protocol::Command;
use leptoskit_plugin_devtools::Hello;
use leptoskit_plugin_devtools::decode_commands;
use leptoskit_plugin_devtools::guest::Devtools;

use super::Agent;
use super::Pending;
use super::Transport;
use super::Wake;
use crate::registry;

thread_local! {
	static AGENT: RefCell<Option<Rc<Agent<Bridge>>>> = const { RefCell::new(None) };
	/// Whether the host was already asked for the plugin.
	static ASKED: Cell<bool> = const { Cell::new(false) };
}

/// Starts the agent when the host offers the `DevTools` plugin, which only a
/// host `leptoskit dev` built does. Returns whether it runs.
pub(crate) fn start() -> bool {
	if AGENT.with_borrow(Option::is_some) {
		return true;
	}

	if ASKED.replace(true) {
		return false;
	}

	let Ok(plugin) = Devtools::new() else {
		return false;
	};
	let agent = Rc::new(Agent::new(Bridge(plugin), Waker::from(Arc::new(Poll))));
	AGENT.set(Some(Rc::clone(&agent)));

	registry::install(Rc::new(Wake(Rc::clone(&agent))));
	agent.start();
	true
}

/// Polls the agent when the host answers one of its calls.
struct Poll;

impl std::task::Wake for Poll {
	fn wake(self: Arc<Self>) {
		self.wake_by_ref();
	}

	fn wake_by_ref(self: &Arc<Self>) {
		// Out of the thread local first: polling may wake again.
		if let Some(agent) = AGENT.with_borrow(Clone::clone) {
			agent.poll();
		}
	}
}

/// The host's `DevTools` plugin.
struct Bridge(Devtools);

impl Transport for Bridge {
	fn hello(&self, hello: &Hello) -> Pending<()> {
		let call = self.0.hello(hello);
		Box::pin(async move { call?.await.map(drop) })
	}

	fn report(&self, payload: &[u8]) -> Pending<()> {
		let call = self.0.report(payload);
		Box::pin(async move { call?.await.map(drop) })
	}

	fn commands(&self) -> Pending<Vec<Command>> {
		let call = self.0.commands();
		Box::pin(async move { decode_commands(&call?.await?) })
	}

	fn wait(&self, ms: u32) -> Pending<()> {
		let call = self.0.wait(ms);
		Box::pin(async move { call?.await.map(drop) })
	}

	fn render(&self) {
		leptoskit_native::guest::render();
	}
}
