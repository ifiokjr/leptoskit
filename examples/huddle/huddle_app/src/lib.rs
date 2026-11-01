//! Huddle's interface: one tree of components for the browser and the
//! native apps' Wasm guest.
//!
//! [`provide`] creates the app's state once, and [`screen`] builds the tree
//! from it whenever something changes: the sign-in screen until someone
//! signs in, then the chat, the board, and the activity panel. The same two
//! functions run in the browser ([`App`]) and in the native runtime
//! (`leptoskit_main`).

pub mod auth;
pub mod huddle;
pub mod room;
pub mod ui;

use leptoskit::leptos::prelude::*;
use leptoskit::prelude::Element;

use crate::auth::SignIn;
use crate::huddle::Huddle;

/// Where the native apps find the server; browsers use the page's own
/// origin. Set `HUDDLE_SERVER` when building for another server, such as
/// the Fly demo, and list it in `[native.plugin-options]`.
pub const SERVER: &str = match option_env!("HUDDLE_SERVER") {
	Some(server) => server,
	None => "http://127.0.0.1:3000",
};

/// The app's state: sign-in, and the signed-in screens once someone is.
#[derive(Clone, Copy)]
pub struct Shell {
	pub sign_in: SignIn,
	/// Created the first time someone is signed in, under the app's own
	/// owner, so its queries outlive any one render.
	pub huddle: RwSignal<Option<Huddle>>,
}

/// Starts the app's shared state in the current reactive owner: the query
/// cache, the sign-in screen, and an effect that builds the signed-in
/// state when someone signs in.
pub fn provide() -> Shell {
	leptoskit::query::QueryClient::new().provide();
	let owner = Owner::current().expect("provide runs inside a reactive owner");
	let shell = Shell {
		sign_in: SignIn::new(),
		huddle: RwSignal::new(None),
	};
	provide_context(shell);

	Effect::new(move |was: Option<Option<bool>>| {
		let now = shell.sign_in.known();

		if now == Some(true) && shell.huddle.get_untracked().is_none() {
			shell.huddle.set(Some(owner.with(Huddle::new)));
		}

		// The live socket keeps the identity it connected with, so the
		// browser starts again, as the new person, after signing in or out.
		let was = was.flatten();

		if let (Some(was), Some(now)) = (was, now)
			&& was != now
		{
			reload();
		}

		now.or(was)
	});

	// Signed out everywhere by an admin: this session no longer holds, so
	// forget it here too.
	Effect::new(move |_| {
		let ended = shell
			.huddle
			.get()
			.and_then(|huddle| huddle.session.data())
			.is_some_and(|session| !session.active);

		if ended {
			shell.sign_in.sign_out();
		}
	});

	shell
}

/// The whole interface, for whoever is (or isn't) signed in.
pub fn screen(shell: &Shell) -> Element {
	let shell = *shell;
	match shell.huddle.get().filter(|_| shell.sign_in.is_signed_in()) {
		Some(huddle) => ui::screen(&shell, &huddle),
		None => auth::screen(shell.sign_in),
	}
}

/// Starts the browser over, so the live socket reconnects as whoever is
/// signed in now. Native apps keep going: their socket signs in with a
/// token when it next connects.
pub fn reload() {
	#[cfg(all(target_arch = "wasm32", feature = "web"))]
	if let Some(window) = web_sys::window() {
		let _ = window.location().reload();
	}
}

/// The browser adapter for the same tree.
#[cfg(feature = "web")]
#[leptos::component]
pub fn App() -> impl IntoView {
	let shell = provide();
	move || leptoskit::web::render(screen(&shell))
}

/// Mounts the app when the browser loads its Wasm module.
#[cfg(all(target_arch = "wasm32", feature = "web"))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
	#[cfg(feature = "devtools")]
	leptoskit::devtools::install(env!("CARGO_PKG_NAME"));
	// The browser talks to the page's own origin, with the session cookie;
	// this configures the RPC client and the live socket too.
	leptoskit::auth::client::configure(leptoskit::auth::client::ClientConfig::new());
	mount_to_body(App);
}

#[cfg(all(target_arch = "wasm32", feature = "native"))]
mod guest;
