//! The arena's scenario app, shared by the browser and the native runners.
//!
//! On launch it reads the scenario the platform passed, runs it on its own
//! clock, and writes one `ARENA` report to the system log. With no scenario
//! it shows a home screen with one button per scenario.

pub mod components;
pub mod report;
pub mod routes;
pub mod services;

#[cfg(target_arch = "wasm32")]
mod host;
#[cfg(target_arch = "wasm32")]
mod scenarios;

#[cfg(target_arch = "wasm32")]
use leptoskit::prelude::Set;
pub use services::Arena;
use services::Scenario;

/// Renders the portable application tree for any host.
pub fn screen(arena: &Arena) -> leptoskit::prelude::Element {
	routes::screen(arena)
}

/// Starts a scenario from the home screen.
pub fn start(arena: &Arena, scenario: Scenario) {
	#[cfg(target_arch = "wasm32")]
	match host::Host::new() {
		Ok(host) => runtime::spawn(scenarios::run(host, arena.clone(), scenario)),
		Err(error) => {
			arena.update(|model| {
				model.phase.set(services::Phase::Failed);
				model.screen = services::Screen::Unsupported(error);
			});
		}
	}

	#[cfg(not(target_arch = "wasm32"))]
	let _ = (arena, scenario);
}

/// The browser and server adapter for the same application tree.
#[cfg(feature = "web")]
#[leptos::component]
pub fn App() -> impl leptos::prelude::IntoView {
	use leptos::prelude::*;

	let changed = RwSignal::new(0_u32);
	let arena =
		Arena::default().on_change(move || changed.update(|value| *value = value.wrapping_add(1)));

	#[cfg(target_arch = "wasm32")]
	runtime::spawn(scenarios::launch(arena.clone()));

	move || {
		changed.get();
		leptoskit::web::render(screen(&arena))
	}
}

/// The complete HTML document the scaffold's SSR server sends. The arena
/// builds the web app in CSR mode (`app.render = "csr"`), so a static
/// `dist/` serves it; the server stays buildable for `render = "ssr"`.
#[cfg(feature = "web")]
pub fn shell(options: leptos::config::LeptosOptions) -> impl leptos::prelude::IntoView {
	use leptos::prelude::*;

	leptos::view! {
		<!DOCTYPE html>
		<html lang="en">
			<head>
				<meta charset="utf-8" />
				<meta name="viewport" content="width=device-width, initial-scale=1" />
				<link rel="icon" href="data:," />
				<title>"arena"</title>
				<HydrationScripts options />
				<style>{include_str!("../../assets/app.css")}</style>
			</head>
			<body>
				<App />
			</body>
		</html>
	}
}

/// Hydrates server markup when the document's hydration script calls it.
#[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
	leptos::mount::hydrate_body(App);
}

/// Mounts the CSR application when the browser loads its Wasm module.
#[cfg(all(target_arch = "wasm32", feature = "web", not(feature = "hydrate")))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start_web() {
	leptos::mount::mount_to_body(App);
}

/// Spawns the scenario drivers on each destination's executor.
#[cfg(target_arch = "wasm32")]
mod runtime {
	use std::future::Future;

	/// The browser's microtask executor.
	#[cfg(feature = "web")]
	pub fn spawn(task: impl Future<Output = ()> + 'static) {
		wasm_bindgen_futures::spawn_local(task);
	}

	#[cfg(feature = "native")]
	pub use native::poll;
	#[cfg(feature = "native")]
	pub use native::spawn;

	/// A single-threaded pool the host drives through `leptoskit_poll` after
	/// every plugin reply. leptoskit has no guest executor of its own.
	#[cfg(feature = "native")]
	mod native {
		use std::cell::RefCell;
		use std::future::Future;

		use futures::executor::LocalPool;
		use futures::task::LocalSpawnExt;

		thread_local! {
			static POOL: RefCell<LocalPool> = RefCell::new(LocalPool::new());
		}

		/// Queues a task and runs it until it waits on the host.
		pub fn spawn(task: impl Future<Output = ()> + 'static) {
			POOL.with_borrow(|pool| pool.spawner().spawn_local(task))
				.expect("the guest executor lives as long as the guest");
			poll();
		}

		/// Runs every task that can make progress.
		pub fn poll() {
			POOL.with_borrow_mut(LocalPool::run_until_stalled);
		}
	}
}

/// Mounts the application in the native Wasm runtime, then reads its scenario.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	let arena = Arena::default().on_change(leptoskit::guest::render);
	let view = arena.clone();
	leptoskit::guest::mount(move || screen(&view));
	runtime::spawn(scenarios::launch(arena));
}

/// Forwards platform-neutral events to the shared tree's handlers.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, ptr: i32, len: i32) -> i32 {
	leptoskit::guest::dispatch(node, kind, ptr, len)
}

/// Resumes the scenario after the host delivers plugin replies.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_poll() {
	runtime::poll();
}

/// Allocates the native runtime's event payload buffer.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_alloc(len: i32) -> i32 {
	leptoskit::guest::alloc(len)
}

/// Reports the native UI bridge ABI implemented by this app.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_abi_version() -> i32 {
	leptoskit::guest::ABI_VERSION as i32
}
