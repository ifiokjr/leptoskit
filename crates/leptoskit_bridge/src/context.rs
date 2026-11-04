//! Platform host context shared with Rust plugins during native app creation.
//!
//! Host runtimes lend existing application-private directories (`data_dir` and
//! `cache_dir`) together with an optional retained platform handle:
//! - On Android, `vm` is the process `JavaVM*` and `context` is a JNI global
//!   reference (`jobject`) to the Android `Context`, released by the host's
//!   callback when the last [`PlatformContext`] clone drops.
//! - On Apple platforms (iOS and macOS), `vm` is a main-thread C resolver
//!   callback ([`AppleOwnerResolver`]) and `context` is a retained
//!   `NativePluginContext` reference whose weak `UIViewController` or
//!   `NSViewController` owner is queried without extending the controller's
//!   lifetime.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

/// Callback invoked when the last [`PlatformContext`] clone drops.
pub type ReleasePlatformContext =
	unsafe extern "C" fn(platform_vm: *mut c_void, platform_context: *mut c_void);

/// Main-thread resolver callback installed by the Apple host in `platform_vm`.
pub type AppleOwnerResolver =
	unsafe extern "C" fn(platform_context: *mut c_void, kind: i32) -> *mut c_void;

/// Kind of Apple view controller lookup requested through [`PlatformContext`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum AppleOwnerKind {
	/// Returns the view controller while its view is attached to a window
	/// (`NativePluginContext.requireAttachedOwner`).
	Attached = 0,
	/// Returns the view controller only when its window or scene is visible and
	/// foreground-active (`NativePluginContext.requirePresentationOwner`).
	Presentation = 1,
}

struct PlatformInner {
	vm: *mut c_void,
	context: *mut c_void,
	release: Option<ReleasePlatformContext>,
}

// SAFETY: The embedding host supplies thread-safe handles (`JavaVM*` + JNI
// global reference on Android, or an atomic reference-counted
// `NativePluginContext` box whose resolver verifies `Thread.isMainThread` on
// Apple) and a thread-safe release callback.
unsafe impl Send for PlatformInner {}
// SAFETY: Shared access only reads raw pointers or invokes thread-checked host
// callbacks; mutable host state is synchronized by JNI or `@MainActor`.
unsafe impl Sync for PlatformInner {}

impl Drop for PlatformInner {
	fn drop(&mut self) {
		if let Some(release) = self.release {
			// SAFETY: Ownership of `(vm, context)` was transferred by the host
			// to be released once when the last `PlatformContext` clone drops.
			unsafe {
				release(self.vm, self.context);
			}
		}
	}
}

/// Shared, reference-counted handle to the embedding platform's native context.
///
/// Cloning a `PlatformContext` is cheap (`Arc`) and keeps the underlying host
/// reference alive until the application and all plugins holding a clone have
/// been dropped.
#[derive(Clone)]
pub struct PlatformContext(Arc<PlatformInner>);

impl fmt::Debug for PlatformContext {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("PlatformContext")
			.field("vm", &self.0.vm)
			.field("context", &self.0.context)
			.field("has_release", &self.0.release.is_some())
			.finish()
	}
}

impl PlatformContext {
	/// Takes ownership of an optional raw platform handle and its release
	/// callback.
	///
	/// Returns `None` when both pointers and `release` are null.
	///
	/// # Safety
	/// When `release` is `Some`, `release(vm, context)` must be safe to call
	/// once from any thread when the last clone of the returned
	/// [`PlatformContext`] drops.
	#[must_use]
	pub unsafe fn from_raw(
		vm: *mut c_void,
		context: *mut c_void,
		release: Option<ReleasePlatformContext>,
	) -> Option<Self> {
		if vm.is_null() && context.is_null() && release.is_none() {
			return None;
		}

		Some(Self(Arc::new(PlatformInner {
			vm,
			context,
			release,
		})))
	}

	/// Returns `(java_vm, android_context_global_ref)` when both pointers were
	/// supplied by the Android JNI host.
	#[must_use]
	pub fn android_handles(&self) -> Option<(*mut c_void, *mut c_void)> {
		(!self.0.vm.is_null() && !self.0.context.is_null()).then_some((self.0.vm, self.0.context))
	}

	/// Resolves the live Apple `UIViewController` or `NSViewController` pointer
	/// on the main thread using the host's `NativePluginContext` rules, without
	/// extending the controller's lifetime.
	///
	/// Returns `None` when called off the main thread, when the view is not in
	/// a qualifying window/scene, or after the host runtime invalidates its
	/// plugin context on close. Only Apple hosts install a resolver, so this is
	/// unavailable elsewhere rather than calling another platform's handle.
	#[cfg(target_vendor = "apple")]
	#[must_use]
	pub fn apple_owner(&self, kind: AppleOwnerKind) -> Option<*mut c_void> {
		if self.0.vm.is_null() || self.0.context.is_null() {
			return None;
		}

		// SAFETY: On Apple hosts `vm` is a C function pointer matching
		// `AppleOwnerResolver` and `context` is the retained
		// `NativePluginContext` opaque pointer.
		let resolve: AppleOwnerResolver = unsafe { std::mem::transmute(self.0.vm) };
		// SAFETY: The resolver checks `Thread.isMainThread` before accessing
		// MainActor state and returns an unretained controller pointer or null.
		let owner = unsafe { resolve(self.0.context, kind as i32) };
		(!owner.is_null()).then_some(owner)
	}

	/// Resolves the attached `UIViewController` or `NSViewController` on the
	/// main thread ([`AppleOwnerKind::Attached`]).
	#[cfg(target_vendor = "apple")]
	#[must_use]
	pub fn apple_attached_owner(&self) -> Option<*mut c_void> {
		self.apple_owner(AppleOwnerKind::Attached)
	}

	/// Resolves the foreground-active presentation `UIViewController` or
	/// `NSViewController` on the main thread
	/// ([`AppleOwnerKind::Presentation`]).
	#[cfg(target_vendor = "apple")]
	#[must_use]
	pub fn apple_presentation_owner(&self) -> Option<*mut c_void> {
		self.apple_owner(AppleOwnerKind::Presentation)
	}
}

/// Native application directories and optional platform host handles supplied
/// by the trusted platform host.
#[derive(Debug, Clone)]
pub struct AppContext {
	/// Existing, application-private persistent data directory.
	pub data_dir: PathBuf,
	/// Existing, application-private cache directory.
	pub cache_dir: PathBuf,
	/// Optional platform-native host handles (`JavaVM`/`Context` on Android,
	/// presentation-owner resolver on Apple).
	pub platform: Option<PlatformContext>,
}

impl AppContext {
	/// Creates an [`AppContext`] with application directories and no platform
	/// handle.
	#[must_use]
	pub fn new(data_dir: PathBuf, cache_dir: PathBuf) -> Self {
		Self {
			data_dir,
			cache_dir,
			platform: None,
		}
	}

	/// Optional platform host handle (`JavaVM`/`Context` on Android,
	/// `NativePluginContext` owner resolver on Apple).
	#[must_use]
	pub fn platform(&self) -> Option<&PlatformContext> {
		self.platform.as_ref()
	}
}
