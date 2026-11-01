//! The offline queue's copy on the device: the mutations and actions
//! dispatched while the connection was down, kept so they still go out if
//! the app closes before it comes back.
//!
//! In the browser it lives in `localStorage`, and in a native app in the
//! store plugin's settings file. A mutation is kept until it answers, with
//! the idempotency key it was dispatched with, so a replay after a restart
//! runs once; an action is forgotten as it goes out, so a restart never
//! sends it twice.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use leptoskit_query::BoxFuture;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

/// The key the queue is kept under.
#[allow(dead_code, reason = "only the browser and native stores name it")]
pub(crate) const STORAGE_KEY: &str = "leptoskit.rpc.offline_queue";

/// A call waiting for the connection, as the device keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueuedCall {
	/// Whether it is a mutation or an action.
	pub kind: QueuedKind,
	/// The operation's name.
	pub name: String,
	/// Its arguments.
	#[serde(default)]
	pub args: Value,
	/// A mutation's idempotency key, which makes its replay run once.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub key: Option<String>,
}

/// What kind of call a [`QueuedCall`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueuedKind {
	/// A mutation: sent again until it answers.
	Mutation,
	/// An action: sent at most once.
	Action,
}

/// Where the offline queue is kept between runs of the app.
///
/// [`ClientConfig::offline_queue`](super::ClientConfig::offline_queue)
/// picks the platform's own; [`MemoryQueue`] keeps it for a test, or for an
/// app that only wants the queue while it runs.
pub trait QueueStore: Send + Sync + 'static {
	/// What was kept, oldest first. Empty when nothing was, or it can't be
	/// read.
	fn load(&self) -> BoxFuture<'static, Vec<QueuedCall>>;

	/// Keeps `calls` in place of what was kept.
	fn save(&self, calls: Vec<QueuedCall>) -> BoxFuture<'static, ()>;
}

/// A queue kept in memory, shared by every clone: what a test hands one
/// client and then another, as an app that restarted would find it.
#[derive(Clone, Debug, Default)]
pub struct MemoryQueue(Arc<Mutex<Vec<QueuedCall>>>);

impl MemoryQueue {
	/// An empty queue.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// What it holds now.
	#[must_use]
	pub fn calls(&self) -> Vec<QueuedCall> {
		self.0
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.clone()
	}
}

impl QueueStore for MemoryQueue {
	fn load(&self) -> BoxFuture<'static, Vec<QueuedCall>> {
		let calls = self.calls();
		Box::pin(std::future::ready(calls))
	}

	fn save(&self, calls: Vec<QueuedCall>) -> BoxFuture<'static, ()> {
		*self.0.lock().unwrap_or_else(PoisonError::into_inner) = calls;
		Box::pin(std::future::ready(()))
	}
}

/// This platform's own store: `localStorage` in the browser, the store
/// plugin in a native app, and none elsewhere.
pub(crate) fn platform_store() -> Option<Arc<dyn QueueStore>> {
	#[cfg(all(
		feature = "web",
		not(feature = "guest"),
		target_arch = "wasm32",
		target_os = "unknown"
	))]
	return Some(Arc::new(browser::LocalStorage));

	#[cfg(all(feature = "guest", target_arch = "wasm32"))]
	return Some(Arc::new(native::Settings));

	#[allow(unreachable_code, reason = "a platform with a store returned it above")]
	None
}

/// The queue in `localStorage`, under its own key, beside the app's own
/// settings.
#[cfg(all(
	feature = "web",
	not(feature = "guest"),
	target_arch = "wasm32",
	target_os = "unknown"
))]
mod browser {
	use leptoskit_query::BoxFuture;

	use super::QueueStore;
	use super::QueuedCall;
	use super::STORAGE_KEY;

	pub(super) struct LocalStorage;

	fn storage() -> Option<web_sys::Storage> {
		web_sys::window()?.local_storage().ok().flatten()
	}

	impl QueueStore for LocalStorage {
		fn load(&self) -> BoxFuture<'static, Vec<QueuedCall>> {
			let calls = storage()
				.and_then(|storage| storage.get_item(STORAGE_KEY).ok().flatten())
				.and_then(|text| serde_json::from_str(&text).ok())
				.unwrap_or_default();
			Box::pin(std::future::ready(calls))
		}

		fn save(&self, calls: Vec<QueuedCall>) -> BoxFuture<'static, ()> {
			if let Some(storage) = storage() {
				if calls.is_empty() {
					let _ = storage.remove_item(STORAGE_KEY);
				} else if let Ok(text) = serde_json::to_string(&calls) {
					let _ = storage.set_item(STORAGE_KEY, &text);
				}
			}

			Box::pin(std::future::ready(()))
		}
	}
}

/// The queue in the store plugin's settings, saved with each change.
#[cfg(all(feature = "guest", target_arch = "wasm32"))]
mod native {
	use leptoskit_plugin_store::Change;
	use leptoskit_plugin_store::Save;
	use leptoskit_plugin_store::guest::Store;
	use leptoskit_query::BoxFuture;

	use super::QueueStore;
	use super::QueuedCall;
	use super::STORAGE_KEY;

	pub(super) struct Settings;

	impl QueueStore for Settings {
		fn load(&self) -> BoxFuture<'static, Vec<QueuedCall>> {
			Box::pin(send_wrapper::SendWrapper::new(async {
				let Ok(store) = Store::new() else {
					return Vec::new();
				};

				match store.get(STORAGE_KEY).await {
					Ok(Some(value)) => serde_json::from_value(value).unwrap_or_default(),
					_ => Vec::new(),
				}
			}))
		}

		fn save(&self, calls: Vec<QueuedCall>) -> BoxFuture<'static, ()> {
			Box::pin(send_wrapper::SendWrapper::new(async move {
				let Ok(store) = Store::new() else {
					return;
				};
				let change = if calls.is_empty() {
					Change::Remove(STORAGE_KEY.to_owned())
				} else {
					match serde_json::to_value(&calls) {
						Ok(value) => Change::Set(STORAGE_KEY.to_owned(), value),
						Err(_) => return,
					}
				};
				let _ = store.apply(&[change], Save::Commit).await;
			}))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_queued_call_round_trips() {
		let call = QueuedCall {
			kind: QueuedKind::Mutation,
			name: "send".to_owned(),
			args: serde_json::json!({ "body": "hi" }),
			key: Some("k".to_owned()),
		};
		let text = serde_json::to_string(&call).unwrap();
		assert_eq!(serde_json::from_str::<QueuedCall>(&text).unwrap(), call);
		assert!(platform_store().is_none());
	}

	#[test]
	fn a_memory_queue_is_shared_by_its_clones() {
		let queue = MemoryQueue::new();
		let other = queue.clone();
		futures::executor::block_on(queue.save(vec![QueuedCall {
			kind: QueuedKind::Action,
			name: "charge".to_owned(),
			args: Value::Null,
			key: None,
		}]));
		assert_eq!(other.calls().len(), 1);
		let loaded = futures::executor::block_on(other.load());
		assert_eq!(loaded, queue.calls());
	}
}
