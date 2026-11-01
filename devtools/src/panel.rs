//! The browser `DevTools` panel: a small shell around the hub's interface.
//!
//! `leptoskit build --platform extension` packages two routes of it, which
//! `[extension.devtools_page]` in `leptoskit.toml` names:
//!
//! - [`REGISTER_ROUTE`]: the browser's developer tools open it out of sight for
//!   each tab they inspect, and it adds the "leptoskit" panel.
//! - [`PANEL_ROUTE`]: the panel. It finds the hub, frames the hub's interface
//!   below a bar with the hub's port, and tells the interface which session the
//!   inspected tab reports.
//!
//! The panel never bundles the interface: everything below the bar is the
//! hub's own page, so it always matches the `leptoskit dev` that serves it,
//! and a panel installed from a store needs no update when the interface
//! changes.
//!
//! [`Shell`] is what the panel shows and [`screen`] draws it. The browser side
//! (the frame, the inspected tab's session, and looking for the hub) runs
//! only in the extension; on the host, for tests, it does nothing.

use leptoskit::prelude::*;
use leptoskit_extension::Surface;
use leptoskit_extension::devtools;
use reactive_graph::signal::ArcRwSignal;
use reactive_graph::traits::Update;
use reactive_graph::traits::With;
use reactive_graph::traits::WithUntracked;

use crate::view::CONTROL;
use crate::view::FIELD;
use crate::view::LINE;
use crate::view::PAGE;
use crate::view::QUIET;

/// The route the developer tools open out of sight; it adds the panel.
pub const REGISTER_ROUTE: &str = "/devtools";

/// The panel's route.
pub const PANEL_ROUTE: &str = "/devtools/panel";

/// The panel's title in the developer tools.
pub const TITLE: &str = "leptoskit";

/// Where the hub's frame starts: just below the bar, which is `h-9`.
pub const FRAME_TOP: &str = "36px";

/// Where the panel remembers the port it was last told, in extension
/// storage.
pub const PORT_KEY: &str = "leptoskit.devtools.port";

/// What answered on the hub's port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hub {
	/// Nothing has answered yet.
	Looking,
	/// The hub answered, running this leptoskit version.
	Found(String),
	/// Nothing that is a hub answers on the port.
	Missing,
}

impl Hub {
	/// What `GET /api/hub` answered: the hub when it names its version.
	#[must_use]
	pub fn from_answer(body: &str) -> Self {
		serde_json::from_str::<serde_json::Value>(body)
			.ok()
			.and_then(|answer| answer["version"].as_str().map(str::to_owned))
			.map_or(Self::Missing, Self::Found)
	}
}

/// What the panel shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
	/// The port the panel looks for the hub on.
	pub port: u16,
	/// The port field's text, which may not be a port yet.
	pub draft: String,
	/// What answered on the port.
	pub hub: Hub,
	/// Whether the panel runs inside the developer tools. Opened as a plain
	/// page, it has no tab to inspect.
	pub inspecting: bool,
}

impl Default for Shell {
	fn default() -> Self {
		Self {
			port: devtools::HUB_PORT,
			draft: devtools::HUB_PORT.to_string(),
			hub: Hub::Looking,
			inspecting: true,
		}
	}
}

impl Shell {
	/// The hub's address, as `leptoskit dev` prints it.
	#[must_use]
	pub fn address(&self) -> String {
		format!("127.0.0.1:{}", self.port)
	}

	/// The bar's one line about the hub.
	#[must_use]
	pub fn status(&self) -> String {
		match &self.hub {
			Hub::Looking => format!("Looking for the hub on {}…", self.address()),
			Hub::Found(version) => format!("leptoskit {version} · {}", self.address()),
			Hub::Missing => format!("No hub on {}", self.address()),
		}
	}
}

thread_local! {
	static SHELL: ArcRwSignal<Shell> = ArcRwSignal::new(Shell::default());
}

/// Reads the shell, so the panel renders again when it changes.
pub fn read<T>(read: impl FnOnce(&Shell) -> T) -> T {
	SHELL.with(|shell| shell.with(read))
}

/// Reads the shell without subscribing to it.
pub fn peek<T>(read: impl FnOnce(&Shell) -> T) -> T {
	SHELL.with(|shell| shell.with_untracked(read))
}

/// Changes the shell.
pub fn update<T>(change: impl FnOnce(&mut Shell) -> T) -> T {
	SHELL.with(|shell| {
		let mut result = None;
		shell.update(|shell| result = Some(change(shell)));
		result.expect("the update ran")
	})
}

/// Edits the port field.
pub fn edit_port(text: &str) {
	update(|shell| text.clone_into(&mut shell.draft));
}

/// Looks for the hub on the port the field names, and remembers it.
pub fn connect() {
	let Some(port) = peek(|shell| devtools::parse_port(&shell.draft)) else {
		return;
	};
	update(|shell| {
		shell.port = port;
		shell.draft = port.to_string();
		shell.hub = Hub::Looking;
	});
	browser::remember(port);
	browser::look_now();
}

/// Looks for the hub again now.
pub fn look_again() {
	update(|shell| shell.hub = Hub::Looking);
	browser::look_now();
}

/// Opens one of the two routes: the panel, or the page that adds it, which
/// shows nothing.
pub fn main(surface: Surface) -> impl Fn() -> Element + Send + Sync + 'static {
	let panel = surface.route() == PANEL_ROUTE;
	if panel {
		browser::start_panel();
	} else {
		browser::add_panel();
	}
	move || if panel { page() } else { View() }
}

/// The panel, from the current state.
#[must_use]
pub fn page() -> Element {
	read(screen)
}

/// The panel for `shell`: the bar, and below it either the hub's frame or
/// what to do while there is no hub.
#[must_use]
pub fn screen(shell: &Shell) -> Element {
	View()
		.class(PAGE)
		.child(bar(shell))
		.children((shell.hub == Hub::Missing).then(|| missing(shell)))
}

fn bar(shell: &Shell) -> Element {
	let typed = devtools::parse_port(&shell.draft);
	View()
		.test_id("bar")
		.class(format!(
			"h-9 flex-row items-center gap-2 overflow-hidden border-b px-2 {LINE}"
		))
		.child(Text(TITLE).class("text-sm font-semibold"))
		.child(
			TextInput()
				.class(FIELD)
				.class("w-20")
				.accessibility_label("Hub port")
				.placeholder(devtools::HUB_PORT.to_string())
				.value(shell.draft.clone())
				.on_change(edit_port),
		)
		.child(
			Button("Connect")
				.class(CONTROL)
				.enabled(typed.is_some())
				.aria_description(if typed.is_some() {
					"Look for the hub on this port."
				} else {
					"Enter a port from 1 to 65535."
				})
				.on_press(connect),
		)
		.child(Status(shell.status()).class(format!("flex-1 {QUIET}")))
}

fn missing(shell: &Shell) -> Element {
	View()
		.class("gap-2 p-4")
		.child(Heading(2, "No DevTools hub is running").class("text-base font-semibold"))
		.child(
			Text(format!(
				"Nothing answers on {}. Run leptoskit dev in your app's folder: it starts the \
				 hub and prints its address, such as [devtools] hub at http://127.0.0.1:{}. If \
				 it prints another port, enter it above.",
				shell.address(),
				devtools::HUB_PORT,
			))
			.class("text-sm"),
		)
		.child(Text("The panel connects as soon as the hub starts.").class(QUIET))
		.children((!shell.inspecting).then(|| {
			Text(
				"This page isn't in the browser's developer tools, so it has no tab to inspect. \
				 Open the developer tools on your app's tab and choose the leptoskit panel.",
			)
			.class(QUIET)
		}))
		.child(
			Button("Look again")
				.class(CONTROL)
				.class("self-start")
				.on_press(look_again),
		)
}

#[cfg(target_arch = "wasm32")]
mod browser {
	use std::cell::RefCell;

	use leptoskit_extension::devtools::HubFrame;
	use leptoskit_extension::devtools::SessionWatch;
	use leptoskit_extension::storage;
	use wasm_bindgen::JsCast;
	use wasm_bindgen::closure::Closure;
	use wasm_bindgen_futures::JsFuture;
	use wasm_bindgen_futures::spawn_local;

	use super::*;

	/// How often the panel looks for the hub again, so that it connects when
	/// `leptoskit dev` starts and notices when it stops.
	const LOOK_EVERY_MS: i32 = 2_000;

	/// How long one look waits for an answer.
	const ANSWER_WITHIN_MS: u32 = 1_500;

	thread_local! {
		static FRAME: RefCell<Option<HubFrame>> = const { RefCell::new(None) };
		static WATCH: RefCell<Option<SessionWatch>> = const { RefCell::new(None) };
	}

	/// Adds the panel to the developer tools. Anywhere else there's nothing
	/// to add it to, which leaves this page as it is: empty.
	pub fn add_panel() {
		spawn_local(async {
			let _ = devtools::create_panel(TITLE, "").await;
		});
	}

	/// Frames the hub, follows the inspected tab's session, and keeps
	/// looking for the hub.
	pub fn start_panel() {
		if let Ok(frame) = HubFrame::new(FRAME_TOP) {
			FRAME.with_borrow_mut(|slot| *slot = Some(frame));
		}
		let inspecting = devtools::inspected_tab().is_ok();
		if !inspecting {
			update(|shell| shell.inspecting = false);
		}
		if inspecting
			&& let Ok(watch) = devtools::watch_session(|session| {
				FRAME.with_borrow(|frame| {
					if let Some(frame) = frame {
						frame.name_session(&session);
					}
				});
			}) {
			WATCH.with_borrow_mut(|slot| *slot = Some(watch));
		}

		spawn_local(async {
			if let Ok(Some(port)) = storage::LOCAL.get::<u16>(PORT_KEY).await
				&& port != 0
			{
				update(|shell| {
					shell.port = port;
					shell.draft = port.to_string();
				});
			}
			look().await;
			let again = Closure::<dyn FnMut()>::new(|| spawn_local(look()));
			if let Some(window) = web_sys::window() {
				let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
					again.as_ref().unchecked_ref(),
					LOOK_EVERY_MS,
				);
			}
			// The panel looks for as long as it's open.
			again.forget();
		});
	}

	/// Remembers the port for the next time the panel opens.
	pub fn remember(port: u16) {
		spawn_local(async move {
			let _ = storage::LOCAL.set(PORT_KEY, &port).await;
		});
	}

	pub fn look_now() {
		spawn_local(look());
	}

	/// Asks the port whether it's the hub, then shows its frame or hides it.
	/// An answer about a port the panel no longer uses is dropped.
	async fn look() {
		let port = peek(|shell| shell.port);
		let hub = ask(port).await;
		let (current, changed) = peek(|shell| (shell.port == port, shell.hub != hub));
		if !current {
			return;
		}
		// Unchanged answers leave the panel alone, so typing in the port field
		// isn't interrupted every two seconds.
		if changed {
			update(|shell| shell.hub = hub.clone());
		}
		FRAME.with_borrow(|frame| {
			if let Some(frame) = frame {
				match hub {
					Hub::Found(_) => frame.show(port),
					Hub::Looking | Hub::Missing => frame.hide(),
				}
			}
		});
	}

	async fn ask(port: u16) -> Hub {
		let Some(window) = web_sys::window() else {
			return Hub::Missing;
		};
		let init = web_sys::RequestInit::new();
		init.set_cache(web_sys::RequestCache::NoStore);
		// A port held by something that never answers mustn't stall the
		// panel.
		init.set_signal(Some(&web_sys::AbortSignal::timeout_with_u32(
			ANSWER_WITHIN_MS,
		)));
		let url = format!("{}/api/hub", devtools::hub_origin(port));
		let Ok(response) = JsFuture::from(window.fetch_with_str_and_init(&url, &init)).await else {
			return Hub::Missing;
		};
		let Ok(response) = response.dyn_into::<web_sys::Response>() else {
			return Hub::Missing;
		};
		if !response.ok() {
			return Hub::Missing;
		}
		let Ok(text) = response.text() else {
			return Hub::Missing;
		};
		JsFuture::from(text)
			.await
			.ok()
			.and_then(|text| text.as_string())
			.map_or(Hub::Missing, |body| Hub::from_answer(&body))
	}
}

/// On the host, for tests, the browser side does nothing.
#[cfg(not(target_arch = "wasm32"))]
mod browser {
	pub fn add_panel() {}

	pub fn start_panel() {}

	pub fn remember(_port: u16) {}

	pub fn look_now() {}
}
