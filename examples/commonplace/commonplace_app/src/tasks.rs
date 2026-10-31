//! Running async work: each host has its own executor.
//!
//! - **The browser** runs tasks on its event loop.
//! - **A native guest** keeps a local pool, which the host drives by calling
//!   `leptoskit_poll` after a plugin replies. After each poll the guest renders
//!   again, because the task may have changed the state.
//! - **Tests on the host** run tasks on Tokio's local set.

use std::future::Future;

/// Starts `task` on this host's executor.
pub fn spawn(task: impl Future<Output = ()> + 'static) {
	imp::spawn(task);
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
mod imp {
	pub fn spawn(task: impl std::future::Future<Output = ()> + 'static) {
		wasm_bindgen_futures::spawn_local(task);
	}
}

#[cfg(all(target_arch = "wasm32", feature = "native", not(feature = "web")))]
mod imp {
	use std::cell::RefCell;

	use futures_executor::LocalPool;
	use futures_util::task::LocalSpawnExt;

	thread_local! {
		static POOL: RefCell<LocalPool> = RefCell::new(LocalPool::new());
	}

	pub fn spawn(task: impl std::future::Future<Output = ()> + 'static) {
		POOL.with_borrow(|pool| pool.spawner().spawn_local(task))
			.expect("the guest's pool is alive");
		poll();
	}

	/// Runs every task until each waits on the host, then renders the
	/// result.
	pub fn poll() {
		// A task that spawns another reaches the pool through `spawn`, which
		// would borrow it again; the pool's spawner queues instead.
		let ran = POOL.try_with(|pool| {
			if let Ok(mut pool) = pool.try_borrow_mut() {
				pool.run_until_stalled();
				true
			} else {
				false
			}
		});
		if ran == Ok(true) {
			leptoskit::guest::render();
		}
	}
}

#[cfg(all(target_arch = "wasm32", feature = "native", not(feature = "web")))]
pub use imp::poll;

#[cfg(not(target_arch = "wasm32"))]
mod imp {
	pub fn spawn(task: impl std::future::Future<Output = ()> + 'static) {
		tokio::task::spawn_local(task);
	}
}
