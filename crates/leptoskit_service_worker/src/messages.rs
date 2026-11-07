//! Messages between the page and its controlling service worker.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::Error;
use crate::Result;
use crate::js::Listener;
use crate::register::controller;
use crate::strategy::WorkerMessage;

/// Posts `message` to the worker controlling this page.
///
/// # Errors
///
/// When no worker controls the page, or outside a browser.
pub fn post_message<M: Serialize>(message: &M) -> Result<()> {
	post_message_to(controller()?, message)
}

fn post_message_to<M: Serialize>(
	worker: Option<crate::register::Worker>,
	message: &M,
) -> Result<()> {
	let worker =
		worker.ok_or_else(|| Error::Unavailable("navigator.serviceWorker.controller".into()))?;
	worker.post_message(message)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
	use super::*;
	use crate::Error;
	use crate::register::Worker;

	#[test]
	fn post_message_requires_a_controller() {
		assert_eq!(
			post_message_to(None, &"ping"),
			Err(Error::Unavailable(
				"navigator.serviceWorker.controller".into()
			))
		);
		assert_eq!(
			post_message_to(Some(Worker { _private: () }), &"ping"),
			Err(Error::Unavailable("ServiceWorker.postMessage".into()))
		);
	}
}

/// Posts a typed [`WorkerMessage`] to the controlling worker.
///
/// # Errors
///
/// When no worker controls the page, or outside a browser.
pub fn send(message: &WorkerMessage) -> Result<()> {
	post_message(message)
}

/// Calls `handler` whenever the controlling worker posts a message to this
/// page, until the returned listener is dropped.
///
/// # Errors
///
/// Outside a secure browser context.
pub fn on_message<M, F>(handler: F) -> Result<Listener>
where
	M: DeserializeOwned + 'static,
	F: FnMut(M) + 'static,
{
	#[cfg(target_arch = "wasm32")]
	{
		use crate::js;
		let container = js::container()?;
		let mut handler = handler;
		js::listen_event(&container, "message", move |event| {
			if let Ok(data) = js::property(&event, "data")
				&& let Ok(message) = js::from_js::<M>(&data)
			{
				handler(message);
			}
		})
	}
	#[cfg(not(target_arch = "wasm32"))]
	{
		let _ = handler;
		Err(Error::Unavailable(
			"navigator.serviceWorker.onmessage".into(),
		))
	}
}

/// Calls `handler` when the controlling worker changes (a new one takes
/// control), until the returned listener is dropped.
///
/// # Errors
///
/// Outside a secure browser context.
pub fn on_controller_change(handler: impl FnMut() + 'static) -> Result<Listener> {
	#[cfg(target_arch = "wasm32")]
	{
		use crate::js;
		let container = js::container()?;
		let mut handler = handler;
		js::listen_event(&container, "controllerchange", move |_| handler())
	}
	#[cfg(not(target_arch = "wasm32"))]
	{
		let _ = handler;
		Err(Error::Unavailable(
			"navigator.serviceWorker.oncontrollerchange".into(),
		))
	}
}
