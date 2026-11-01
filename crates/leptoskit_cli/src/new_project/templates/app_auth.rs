//! One application shared by the browser, server, and native runners.

pub mod auth;
pub mod components;
pub mod routes;
pub mod services;

pub use services::Counter;

/// Where native apps find the server; browsers use the page's own origin.
/// An Android emulator reaches the computer running `leptoskit dev` at
/// `http://10.0.2.2:3000`. Change this to your site's origin before you ship.
pub const SERVER: &str = "http://localhost:3000";

/// Renders the portable application tree for any host: the sign-in screen
/// until someone signs in, then the counter.
///
/// It reads the sign-in screens from context, so call it inside the
/// reactive owner where [`auth::provide`] ran.
pub fn screen(state: &Counter) -> leptoskit::prelude::Element {
	auth::screens().gate(|| routes::home::screen(state))
}

/// Starts the app's shared state in the current reactive owner: the query
/// cache the sign-in client keeps the session in, and the sign-in screens.
pub fn provide() {
	leptoskit::query::QueryClient::new().provide();
	auth::provide();
}

/// The browser and server adapter for the same application tree.
#[cfg(feature = "web")]
#[leptos::component]
pub fn App() -> impl leptos::prelude::IntoView {
	use leptos::prelude::*;

	provide();
	let changed = RwSignal::new(0_u32);
	let state = Counter::default()
		.on_change(move || changed.update(|value| *value = value.wrapping_add(1)));

	move || {
		changed.get();
		leptoskit::web::render(screen(&state))
	}
}

/// The complete HTML document sent by the SSR server.
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
				<title>"__NAME__"</title>
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
	#[cfg(feature = "devtools")]
	leptoskit::devtools::install(env!("CARGO_PKG_NAME"));
	// The browser's sign-in client talks to the page's own origin.
	leptoskit::auth::client::configure(leptoskit::auth::client::ClientConfig::new());
	leptos::mount::hydrate_body(App);
}

/// Mounts the CSR application when the browser loads its Wasm module.
#[cfg(all(target_arch = "wasm32", feature = "web", not(feature = "hydrate")))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
	#[cfg(feature = "devtools")]
	leptoskit::devtools::install(env!("CARGO_PKG_NAME"));
	leptoskit::auth::client::configure(leptoskit::auth::client::ClientConfig::new());
	leptos::mount::mount_to_body(App);
}

/// The native runtime has no async executor of its own, so the app keeps
/// one: tasks such as sign-in requests run each time the host polls.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
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

/// Mounts the same application in the native Wasm runtime.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	tasks::start();
	// A native app has to be told where its server is.
	leptoskit::auth::client::configure(leptoskit::auth::client::ClientConfig::new().origin(SERVER));
	let owner = leptoskit::leptos::prelude::Owner::new();
	owner.with(provide);
	let state = Counter::default();
	leptoskit::guest::mount(move || owner.with(|| screen(&state)));
	tasks::poll();
}

/// Forwards platform-neutral events to the shared tree's handlers, then runs
/// the tasks they started.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, ptr: i32, len: i32) -> i32 {
	let handled = leptoskit::guest::dispatch(node, kind, ptr, len);
	tasks::poll();
	handled
}

/// Runs the tasks a plugin reply woke, such as a sign-in request's answer.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_poll() {
	tasks::poll();
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
