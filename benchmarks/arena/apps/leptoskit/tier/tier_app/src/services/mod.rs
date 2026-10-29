//! Application state shared by every destination.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;

use crate::routes::Model;

/// The app's state: the current route and each screen's own state. Each
/// destination decides how a change reaches the screen.
#[derive(Clone, Default)]
pub struct Store {
	model: Arc<Mutex<Model>>,
	on_change: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Store {
	/// Registers a browser render notification without coupling state to its
	/// UI. Native apps render again after every event on their own.
	#[must_use]
	pub fn on_change(mut self, notify: impl Fn() + Send + Sync + 'static) -> Self {
		self.on_change = Some(Arc::new(notify));
		self
	}

	/// Locks the model for a render.
	pub fn read(&self) -> MutexGuard<'_, Model> {
		self.model
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
	}

	/// Changes the model, then notifies the browser.
	pub fn update(&self, change: impl FnOnce(&mut Model)) {
		change(&mut self.read());

		if let Some(notify) = &self.on_change {
			notify();
		}
	}
}
