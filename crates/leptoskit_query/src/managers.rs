//! Window focus and network status.
//!
//! Both managers are process-wide, like TanStack's `focusManager` and
//! `onlineManager`: the browser has one window and one network. They detect
//! state from the browser when there is one and can be overridden anywhere,
//! which is how tests and native shells drive them.

use std::sync::LazyLock;

use futures::channel::oneshot;
use leptos::prelude::*;
use parking_lot::Mutex;

use crate::listeners::Listeners;
use crate::listeners::Subscription;

static FOCUSED: LazyLock<ArcRwSignal<Option<bool>>> = LazyLock::new(|| ArcRwSignal::new(None));
static FOCUS_LISTENERS: LazyLock<Listeners<bool>> = LazyLock::new(Listeners::default);

static ONLINE: LazyLock<ArcRwSignal<bool>> = LazyLock::new(|| ArcRwSignal::new(true));
static ONLINE_LISTENERS: LazyLock<Listeners<bool>> = LazyLock::new(Listeners::default);
static ONLINE_WAITERS: Mutex<Vec<oneshot::Sender<()>>> = Mutex::new(Vec::new());

/// Whether the window is focused.
///
/// In the browser this follows `document.visibilityState`; elsewhere the
/// window counts as focused until [`set_focused`](Self::set_focused) says
/// otherwise.
#[derive(Debug, Clone, Copy)]
pub struct FocusManager;

impl FocusManager {
	/// Whether the window is focused; reactive.
	#[must_use]
	pub fn is_focused() -> bool {
		FOCUSED.get().unwrap_or_else(detect_focused)
	}

	/// Whether the window is focused, without subscribing.
	#[must_use]
	pub fn is_focused_untracked() -> bool {
		FOCUSED.get_untracked().unwrap_or_else(detect_focused)
	}

	/// Overrides focus detection; `None` returns to automatic detection.
	///
	/// Subscribers are told whenever the effective value changes, and every
	/// query that refetches on focus does so when it becomes `true`.
	pub fn set_focused(focused: Option<bool>) {
		let before = Self::is_focused_untracked();
		FOCUSED.set(focused);
		let after = Self::is_focused_untracked();
		if before != after {
			FOCUS_LISTENERS.emit(&after);
		}
	}

	/// Calls `listener` with the new value whenever focus changes.
	pub fn subscribe(listener: impl Fn(bool) + Send + Sync + 'static) -> Subscription {
		FOCUS_LISTENERS.subscribe(move |focused| listener(*focused))
	}

	#[cfg(target_arch = "wasm32")]
	fn notify_browser_change() {
		if FOCUSED.get_untracked().is_none() {
			FOCUS_LISTENERS.emit(&detect_focused());
		}
	}
}

/// Whether the network is reachable.
///
/// In the browser this follows `navigator.onLine` plus the `online` and
/// `offline` events; elsewhere the network counts as online until
/// [`set_online`](Self::set_online) says otherwise.
#[derive(Debug, Clone, Copy)]
pub struct OnlineManager;

impl OnlineManager {
	/// Whether the network is online; reactive.
	#[must_use]
	pub fn is_online() -> bool {
		ONLINE.get()
	}

	/// Whether the network is online, without subscribing.
	#[must_use]
	pub fn is_online_untracked() -> bool {
		ONLINE.get_untracked()
	}

	/// Sets the network status.
	///
	/// Going online resumes every fetch and mutation paused by
	/// [`NetworkMode::Online`](crate::NetworkMode) and refetches every query
	/// that refetches on reconnect.
	pub fn set_online(online: bool) {
		if ONLINE.get_untracked() == online {
			return;
		}
		ONLINE.set(online);
		if online {
			let waiters = std::mem::take(&mut *ONLINE_WAITERS.lock());
			for waiter in waiters {
				let _ = waiter.send(());
			}
		}
		ONLINE_LISTENERS.emit(&online);
	}

	/// Calls `listener` with the new value whenever the status changes.
	pub fn subscribe(listener: impl Fn(bool) + Send + Sync + 'static) -> Subscription {
		ONLINE_LISTENERS.subscribe(move |online| listener(*online))
	}

	/// Resolves immediately if online, otherwise when the network returns.
	pub async fn wait_until_online() {
		let receiver = {
			let mut waiters = ONLINE_WAITERS.lock();
			if ONLINE.get_untracked() {
				return;
			}
			let (sender, receiver) = oneshot::channel();
			waiters.push(sender);
			receiver
		};
		let _ = receiver.await;
	}
}

#[cfg(target_arch = "wasm32")]
fn detect_focused() -> bool {
	web_sys::window()
		.and_then(|window| window.document())
		.is_none_or(|document| document.visibility_state() != web_sys::VisibilityState::Hidden)
}

#[cfg(not(target_arch = "wasm32"))]
fn detect_focused() -> bool {
	true
}

/// Wires the managers to the browser's `visibilitychange`, `online`, and
/// `offline` events. Idempotent; a no-op outside a browser window.
pub fn install_browser_listeners() {
	#[cfg(target_arch = "wasm32")]
	{
		use std::sync::OnceLock;

		use wasm_bindgen::JsCast;
		use wasm_bindgen::closure::Closure;

		static INSTALLED: OnceLock<()> = OnceLock::new();
		let Some(window) = web_sys::window() else {
			return;
		};
		INSTALLED.get_or_init(|| {
			OnlineManager::set_online(window.navigator().on_line());

			let on_online = Closure::<dyn Fn()>::new(|| OnlineManager::set_online(true));
			let on_offline = Closure::<dyn Fn()>::new(|| OnlineManager::set_online(false));
			let on_visibility = Closure::<dyn Fn()>::new(FocusManager::notify_browser_change);
			let _ = window
				.add_event_listener_with_callback("online", on_online.as_ref().unchecked_ref());
			let _ = window
				.add_event_listener_with_callback("offline", on_offline.as_ref().unchecked_ref());
			if let Some(document) = window.document() {
				let _ = document.add_event_listener_with_callback(
					"visibilitychange",
					on_visibility.as_ref().unchecked_ref(),
				);
			}
			// The listeners live as long as the page.
			on_online.forget();
			on_offline.forget();
			on_visibility.forget();
		});
	}
}
