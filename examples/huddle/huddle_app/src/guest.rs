//! The native runtime's entry points: the C names the host calls to start
//! the app, hand it events, and let it run its tasks.

#![allow(unsafe_code)] // Exported C names form the native runtime's entry points.

use leptoskit::leptos::prelude::Owner;

use crate::SERVER;
use crate::provide;
use crate::screen;

/// The native runtime has no async executor of its own, so the app keeps
/// one: tasks such as calls and socket frames run each time the host polls.
mod tasks {
	use std::cell::RefCell;

	use futures::executor::LocalPool;
	use futures::executor::LocalSpawner;
	use futures::task::LocalSpawnExt as _;

	thread_local! {
		static POOL: RefCell<LocalPool> = RefCell::new(LocalPool::new());
		static SPAWNER: LocalSpawner = POOL.with_borrow(LocalPool::spawner);
	}

	struct Pool;

	impl any_spawner::CustomExecutor for Pool {
		fn spawn(&self, task: any_spawner::PinnedFuture<()>) {
			self.spawn_local(task);
		}

		fn spawn_local(&self, task: any_spawner::PinnedLocalFuture<()>) {
			// Only fails once the pool is gone, when nothing runs any more.
			let _ = SPAWNER.with(|spawner| spawner.spawn_local(task));
		}

		fn poll_local(&self) {}
	}

	/// Installs the executor; the first call wins.
	pub fn start() {
		// Take the spawner now: tasks spawned while the pool runs can't borrow
		// it then.
		SPAWNER.with(|_| ());
		let _ = any_spawner::Executor::init_local_custom_executor(Pool);
	}

	/// Runs every task that can make progress, then renders what changed.
	pub fn poll() {
		POOL.with_borrow_mut(LocalPool::run_until_stalled);
		leptoskit::guest::render();
	}
}

/// Mounts the same app in the native Wasm runtime.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	tasks::start();
	// A native app has to be told where its server is. This configures the
	// RPC client and the live socket too, signed with the session's token.
	leptoskit::auth::client::configure(leptoskit::auth::client::ClientConfig::new().origin(SERVER));
	let owner = Owner::new();
	let shell = owner.with(provide);
	leptoskit::guest::mount(move || owner.with(|| screen(&shell)));
	tasks::poll();
}

/// Forwards platform-neutral events to the tree's handlers, then runs the
/// tasks they started.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, ptr: i32, len: i32) -> i32 {
	let handled = leptoskit::guest::dispatch(node, kind, ptr, len);
	tasks::poll();
	handled
}

/// Runs the tasks a plugin reply woke, such as a socket frame or a call's
/// answer.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_poll() {
	tasks::poll();
}

/// Allocates the native runtime's event payload buffer.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_alloc(len: i32) -> i32 {
	leptoskit::guest::alloc(len)
}

/// Reports the native UI bridge ABI this app implements.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_abi_version() -> i32 {
	leptoskit::guest::ABI_VERSION as i32
}
