//! Reactive clipboard text over the plugin's sources.
//!
//! [`use_clipboard`] mirrors the platform clipboard in a signal: read, write,
//! and clear actions update the signal only after the platform operation
//! succeeds. The host executor owns each action's future; nothing is spawned
//! internally.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use leptoskit_bridge::Error;
use reactive_graph::prelude::*;
use reactive_graph::signal::RwSignal;

/// One source operation, polled by the caller's executor.
type SourceFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// An asynchronous clipboard action the caller awaits or schedules on its
/// host executor.
pub type ClipboardAction =
	Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<(), Error>> + 'static>> + Send + Sync>;

/// An asynchronous clipboard writer taking the new text.
pub type ClipboardWriter =
	Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<(), Error>> + 'static>> + Send + Sync>;

mod private {
	/// Seals [`Source`](super::Source) to this crate's implementations.
	pub trait Sealed {}
}

/// Clipboard sources usable with [`use_clipboard`]: the interpreter's client
/// and the browser API. Sealed; test doubles live in this crate.
pub trait Source: private::Sealed + Send + Sync + 'static {
	/// Reads UTF-8 text.
	fn read_text(&self) -> SourceFuture<'_, Result<String, Error>>;
	/// Writes UTF-8 text.
	fn write_text(&self, text: &str) -> SourceFuture<'_, Result<(), Error>>;
	/// Clears the clipboard.
	fn clear(&self) -> SourceFuture<'_, Result<(), Error>>;
}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
impl private::Sealed for crate::guest::Clipboard {}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
impl Source for crate::guest::Clipboard {
	fn read_text(&self) -> SourceFuture<'_, Result<String, Error>> {
		let client = *self;
		Box::pin(async move { client.read_text().await })
	}

	fn write_text(&self, text: &str) -> SourceFuture<'_, Result<(), Error>> {
		let client = *self;
		let text = text.to_owned();
		Box::pin(async move { client.write_text(&text).await })
	}

	fn clear(&self) -> SourceFuture<'_, Result<(), Error>> {
		let client = *self;
		Box::pin(async move { client.clear().await })
	}
}

/// Browser Async Clipboard API source; permission denials surface as errors.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
#[derive(Debug, Clone, Copy)]
pub struct WebClipboard;

#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
impl private::Sealed for WebClipboard {}

#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
impl Source for WebClipboard {
	fn read_text(&self) -> SourceFuture<'_, Result<String, Error>> {
		Box::pin(crate::web::read_text())
	}

	fn write_text(&self, text: &str) -> SourceFuture<'_, Result<(), Error>> {
		let text = text.to_owned();
		Box::pin(async move { crate::web::write_text(&text).await })
	}

	fn clear(&self) -> SourceFuture<'_, Result<(), Error>> {
		Box::pin(crate::web::clear())
	}
}

impl<S: Source> private::Sealed for Arc<S> {}

impl<S: Source> Source for Arc<S> {
	fn read_text(&self) -> SourceFuture<'_, Result<String, Error>> {
		(**self).read_text()
	}

	fn write_text(&self, text: &str) -> SourceFuture<'_, Result<(), Error>> {
		(**self).write_text(text)
	}

	fn clear(&self) -> SourceFuture<'_, Result<(), Error>> {
		(**self).clear()
	}
}

/// Reactive clipboard text.
pub struct UseClipboardReturn {
	/// The last successfully read or written text. Writes update it only
	/// after the platform accepts them.
	pub text: RwSignal<String>,
	/// Reads the platform clipboard into `text`.
	pub read: ClipboardAction,
	/// Writes text to the platform clipboard, then mirrors it in `text`.
	pub write: ClipboardWriter,
	/// Clears the platform clipboard, then empties `text`.
	pub clear: ClipboardAction,
}

/// Mirrors the platform clipboard in a signal inside a Leptos owner.
///
/// Each action returns a future the caller drives; a failed operation
/// leaves the signal untouched. If the owner is disposed while an operation
/// is in flight, the platform effect still completes and the signal update
/// is skipped.
pub fn use_clipboard(source: impl Source) -> UseClipboardReturn {
	let text = RwSignal::new(String::new());
	let publish = Arc::new(move |next: String| {
		if !text.is_disposed() {
			text.try_set(next);
		}
	});

	let read_source = Arc::new(source);
	let write_source = Arc::clone(&read_source);
	let clear_source = Arc::clone(&read_source);
	let read_publish = Arc::clone(&publish);
	let clear_publish = Arc::clone(&publish);

	UseClipboardReturn {
		text,
		read: Arc::new(move || {
			let source = Arc::clone(&read_source);
			let publish = Arc::clone(&read_publish);
			Box::pin(async move {
				let next = source.read_text().await?;
				publish(next);
				Ok(())
			})
		}),
		write: Arc::new(move |next| {
			let source = Arc::clone(&write_source);
			let publish = Arc::clone(&publish);
			Box::pin(async move {
				source.write_text(&next).await?;
				publish(next);
				Ok(())
			})
		}),
		clear: Arc::new(move || {
			let source = Arc::clone(&clear_source);
			let publish = Arc::clone(&clear_publish);
			Box::pin(async move {
				source.clear().await?;
				publish(String::new());
				Ok(())
			})
		}),
	}
}

#[cfg(all(not(target_arch = "wasm32"), test))]
mod tests {
	use std::sync::Mutex;
	use std::sync::atomic::AtomicBool;
	use std::sync::atomic::Ordering;

	use futures::executor::block_on;
	use leptoskit_bridge::Error;
	use reactive_graph::owner::Owner;
	use reactive_graph::prelude::Get;

	use super::Source;
	use super::use_clipboard;

	/// In-memory clipboard with an optional failure switch.
	struct Fake {
		content: Mutex<String>,
		failing: AtomicBool,
	}

	impl Fake {
		fn new() -> Self {
			Self {
				content: Mutex::new("platform text".to_owned()),
				failing: AtomicBool::new(false),
			}
		}
	}

	impl super::private::Sealed for Fake {}

	impl Source for Fake {
		fn read_text(&self) -> super::SourceFuture<'_, Result<String, Error>> {
			if self.failing.load(Ordering::Relaxed) {
				return Box::pin(std::future::ready(Err(Error::PermissionDenied)));
			}
			Box::pin(std::future::ready(Ok(self.content.lock().unwrap().clone())))
		}

		fn write_text(&self, text: &str) -> super::SourceFuture<'_, Result<(), Error>> {
			if self.failing.load(Ordering::Relaxed) {
				return Box::pin(std::future::ready(Err(Error::PermissionDenied)));
			}
			text.clone_into(&mut self.content.lock().unwrap());
			Box::pin(std::future::ready(Ok(())))
		}

		fn clear(&self) -> super::SourceFuture<'_, Result<(), Error>> {
			if self.failing.load(Ordering::Relaxed) {
				return Box::pin(std::future::ready(Err(Error::PermissionDenied)));
			}
			self.content.lock().unwrap().clear();
			Box::pin(std::future::ready(Ok(())))
		}
	}

	#[test]
	fn actions_mirror_successful_platform_operations() {
		let owner = Owner::new();
		let hook = owner.with(|| use_clipboard(Fake::new()));
		assert_eq!(hook.text.get(), "");
		block_on((hook.read)()).unwrap();
		assert_eq!(hook.text.get(), "platform text");
		block_on((hook.write)("copied".to_owned())).unwrap();
		assert_eq!(hook.text.get(), "copied");
		block_on((hook.clear)()).unwrap();
		assert_eq!(hook.text.get(), "");
		owner.cleanup();
	}

	#[test]
	fn failed_operations_leave_the_signal_untouched() {
		let owner = Owner::new();
		let fake = std::sync::Arc::new(Fake::new());
		let hook = owner.with(|| use_clipboard(std::sync::Arc::clone(&fake)));
		block_on((hook.read)()).unwrap();
		fake.failing.store(true, Ordering::Relaxed);
		assert_eq!(
			block_on((hook.write)("denied".to_owned())).unwrap_err(),
			Error::PermissionDenied
		);
		assert_eq!(hook.text.get(), "platform text");
		owner.cleanup();
	}

	#[test]
	fn disposal_after_an_operation_skips_the_signal_update() {
		let owner = Owner::new();
		let hook = owner.with(|| use_clipboard(Fake::new()));
		owner.cleanup();
		block_on((hook.write)("late".to_owned())).unwrap();
	}
}
