#![allow(unsafe_code)] // Exported C names form the native runtime's entry points.

//! Commonplace's app: the screens the website and the native apps share.
//!
//! `src/routes/` holds one file per screen, and the router plugin writes
//! `src/routes.rs` from it. [`store::Store`] holds what the screens show, and
//! [`api::Api`] reaches the server over each host's [`transport`].

pub mod api;
pub mod browser;
#[cfg(feature = "extension")]
pub mod extension;
pub mod routes;
pub mod session;
mod sign_in;
pub mod store;
pub mod tasks;
pub mod transport;
pub mod ui;

/// The server the native apps reach, set at build time with
/// `COMMONPLACE_SERVER`. The default is the server `leptoskit dev` starts on
/// this machine, which the iOS simulator and the macOS app reach directly
/// and an Android emulator reaches after `adb reverse tcp:3000 tcp:3000`.
pub const SERVER: &str = match option_env!("COMMONPLACE_SERVER") {
	Some(server) => server,
	None => "http://127.0.0.1:3000",
};

/// The website: the address bar picks the first screen, and links and the
/// back button move through history.
#[cfg(all(feature = "web", not(feature = "extension"), target_arch = "wasm32"))]
#[leptos::component]
pub fn App() -> impl leptos::prelude::IntoView {
	store::install(api::Api::new(transport::Fetch));
	store::Store::current().start(None);
	let navigation = routes::Navigation::from_browser()
		.unwrap_or_else(|_| routes::Navigation::new(routes::Route::Library));
	move || leptoskit::web::render(navigation.render())
}

/// Mounts the website when the page loads its Wasm module.
#[cfg(all(target_arch = "wasm32", feature = "web", not(feature = "extension")))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
	leptos::mount::mount_to_body(App);
}

// The extension's surfaces and worker; see `extension`.
#[cfg(feature = "extension")]
leptoskit::extension::entry!(main: extension::main, background: extension::background);

/// Mounts the same screens in the native Wasm runtime, signed in with the
/// token the secure store kept from last time.
#[cfg(all(target_arch = "wasm32", feature = "native", not(feature = "web")))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	store::install(api::Api::new(transport::Plugin {
		server: SERVER.to_owned(),
	}));
	let navigation = routes::Navigation::new(routes::Route::Library);
	leptoskit::guest::mount(move || navigation.render());
	// After mounting: a finished task renders again.
	tasks::spawn(async {
		let token = session::kept().await;
		store::Store::current().start(token);
	});
}

/// Forwards platform-neutral events to the shared tree's handlers.
#[cfg(all(target_arch = "wasm32", feature = "native", not(feature = "web")))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, ptr: i32, len: i32) -> i32 {
	let handled = leptoskit::guest::dispatch(node, kind, ptr, len);
	tasks::poll();
	handled
}

/// Runs the tasks a plugin reply woke, then renders what they changed.
#[cfg(all(target_arch = "wasm32", feature = "native", not(feature = "web")))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_poll() {
	tasks::poll();
}

/// Allocates the native runtime's event payload buffer.
#[cfg(all(target_arch = "wasm32", feature = "native", not(feature = "web")))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_alloc(len: i32) -> i32 {
	leptoskit::guest::alloc(len)
}

/// Reports the native UI bridge ABI implemented by this app.
#[cfg(all(target_arch = "wasm32", feature = "native", not(feature = "web")))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_abi_version() -> i32 {
	leptoskit::guest::ABI_VERSION as i32
}
