use crate::Error;
use crate::EventBatch;

/// Unsupported platform placeholder; construction always returns Unavailable.
pub struct GlobalShortcuts {
	_private: (),
}
impl GlobalShortcuts {
	/// Browser/mobile global shortcuts are unavailable.
	pub fn new() -> Result<Self, Error> {
		Err(Error::Unavailable)
	}

	/// Browser/mobile global shortcuts are unavailable.
	pub fn with_waker(_wake: impl Fn() + Send + Sync + 'static) -> Result<Self, Error> {
		Err(Error::Unavailable)
	}

	/// Global shortcuts are unavailable.
	pub fn register(&self, _accelerator: &str) -> Result<Registration, Error> {
		Err(Error::Unavailable)
	}

	/// No native manager can be created on this target.
	pub fn drain(&self) -> EventBatch {
		EventBatch::default()
	}

	/// No native manager can be created on this target.
	pub fn take_cleanup_error(&self) -> Option<Error> {
		None
	}
}

/// A native guard cannot be constructed on unsupported platforms.
pub struct Registration {
	id: u32,
}
impl Registration {
	/// Identifier of an owned registration.
	pub fn id(&self) -> u32 {
		self.id
	}

	/// Global shortcuts are unavailable.
	pub fn unregister(&mut self) -> Result<(), Error> {
		Err(Error::Unavailable)
	}
}
