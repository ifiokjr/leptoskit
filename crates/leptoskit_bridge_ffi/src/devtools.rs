//! `DevTools` for apps `leptoskit dev` launched: the hub they report to, and
//! what the hub asks them to show.
//!
//! An app reports only when its host was built with the `devtools` feature,
//! which only `leptoskit dev` turns on, and only in a debug build: like
//! [`crate::development::lk_dev_open`], a release build refuses. The hub's
//! address comes from `LEPTOSKIT_DEVTOOLS` on a desktop and in the iOS
//! Simulator, and from [`lk_devtools_hub`] on Android, which reads it from
//! the Activity's intent. The first app created after that starts the
//! process's one session, and every later app, such as a guest reloaded by
//! `leptoskit dev`, reports through it.
//!
//! Without the feature these functions do nothing, so platform hosts link
//! the same symbols in every build.

/// A node the hub asked the app to highlight, for the platform to draw over.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct DevtoolsFrame {
	/// The node, as render transactions and events name it.
	pub node: usize,
	/// The left edge, in layout units from the root's.
	pub x: f32,
	/// The top edge, in layout units from the root's.
	pub y: f32,
	/// The width, in layout units.
	pub width: f32,
	/// The height, in layout units.
	pub height: f32,
}

/// Points the apps created after this call at the hub `leptoskit dev` runs,
/// reporting as the app `app`. Android calls it with its intent extras before
/// creating the first app; desktops and the iOS Simulator read the
/// environment instead. Returns 0, or -1 when this build has no `DevTools`, is
/// a release build, or `url` is not a loopback hub.
///
/// # Safety
/// `url` and `app` address their declared lengths during this call, and are
/// null only when empty.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_devtools_hub(
	url: *const u8,
	url_len: usize,
	app: *const u8,
	app_len: usize,
) -> i32 {
	// SAFETY: The caller lends both inputs for this call.
	let (url, app) = unsafe { (text(url, url_len), text(app, app_len)) };

	match (url, app) {
		(Some(url), Some(app)) if configure(url, app) => 0,
		_ => -1,
	}
}

/// Writes the node the hub asked to highlight, with its frame, and returns
/// 1; returns 0 when nothing is highlighted or the node is not on screen,
/// and -1 for a null `out`. Any thread.
///
/// # Safety
/// `out` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_devtools_highlight(out: *mut DevtoolsFrame) -> i32 {
	if out.is_null() {
		return -1;
	}

	match highlighted() {
		Some(frame) => {
			// SAFETY: The caller lends writable output storage.
			unsafe { out.write(frame) };
			1
		}
		None => 0,
	}
}

/// Whether the hub is waiting for a tap to pick a node (1) or not (0). The
/// next press goes to the hub instead of the app. Any thread.
#[unsafe(no_mangle)]
pub extern "C" fn lk_devtools_picking() -> i32 {
	i32::from(picking())
}

/// Borrows bounded UTF-8 input.
///
/// # Safety
/// `data` addresses `len` readable bytes, and is null only when `len` is 0.
unsafe fn text<'a>(data: *const u8, len: usize) -> Option<&'a str> {
	// SAFETY: The caller lends the bounded input for this call.
	let bytes = unsafe { crate::development::input(data, len, 4096) }.ok()?;
	std::str::from_utf8(bytes).ok()
}

#[cfg(all(feature = "devtools", not(target_arch = "wasm32")))]
use agent::configure;
#[cfg(all(feature = "devtools", not(target_arch = "wasm32")))]
use agent::highlighted;
#[cfg(all(feature = "devtools", not(target_arch = "wasm32")))]
use agent::picking;
#[cfg(all(feature = "devtools", not(target_arch = "wasm32")))]
pub(crate) use agent::reporter;

#[cfg(all(feature = "devtools", not(target_arch = "wasm32")))]
mod agent {
	use std::sync::Mutex;
	use std::sync::OnceLock;
	use std::sync::PoisonError;

	use leptoskit_devtools::Reporter;
	use leptoskit_devtools_protocol::APP_ENV;
	use leptoskit_devtools_protocol::Capability;
	use leptoskit_devtools_protocol::HUB_ENV;
	use leptoskit_devtools_protocol::Hello;
	use leptoskit_devtools_protocol::Platform;

	use super::DevtoolsFrame;

	/// The hub and app name a platform host passed, which replace the
	/// environment's.
	static CONFIGURED: Mutex<Option<(String, String)>> = Mutex::new(None);

	/// The process's report, which the first app created starts when there
	/// is a hub to report to.
	static REPORTER: OnceLock<Option<Reporter>> = OnceLock::new();

	pub(super) fn configure(url: &str, app: &str) -> bool {
		if !cfg!(debug_assertions) || leptoskit_devtools::loopback_hub(url).is_none() {
			return false;
		}

		*CONFIGURED.lock().unwrap_or_else(PoisonError::into_inner) =
			Some((url.to_owned(), app.to_owned()));
		true
	}

	/// The process's report, started on first use when there is a hub.
	pub(crate) fn reporter() -> Option<&'static Reporter> {
		REPORTER.get_or_init(start).as_ref()
	}

	pub(super) fn highlighted() -> Option<DevtoolsFrame> {
		let (node, frame) = REPORTER.get()?.as_ref()?.highlight()?.node?;
		Some(DevtoolsFrame {
			node: usize::try_from(node).ok()?,
			x: frame.x,
			y: frame.y,
			width: frame.width,
			height: frame.height,
		})
	}

	pub(super) fn picking() -> bool {
		REPORTER
			.get()
			.and_then(Option::as_ref)
			.is_some_and(Reporter::picking)
	}

	fn start() -> Option<Reporter> {
		let configured = CONFIGURED
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.clone();
		let (url, app) = configured.or_else(|| {
			let url = std::env::var(HUB_ENV).ok()?;
			let app = std::env::var(APP_ENV).unwrap_or_else(|_| "app".to_owned());
			Some((url, app))
		})?;

		if !cfg!(debug_assertions) {
			eprintln!("leptoskit devtools: only debug builds report to a hub; ignoring {HUB_ENV}");
			return None;
		}

		let Some(hub) = leptoskit_devtools::loopback_hub(&url) else {
			eprintln!("leptoskit devtools: {url:?} is not a loopback hub, so nothing is reported");
			return None;
		};
		let hello = Hello::new(
			app,
			platform(),
			// The iOS Simulator names the device it simulates.
			std::env::var("SIMULATOR_DEVICE_NAME").ok(),
			[
				Capability::Tree,
				Capability::Commits,
				Capability::Picking,
				Capability::Profiling,
			],
		);
		// Each app's guest reports its own values through the `DevTools`
		// plugin, and says what it feeds before the session opens.
		Reporter::with_guests(hub, hello)
			.inspect_err(|error| eprintln!("leptoskit devtools: cannot start reporting: {error}"))
			.ok()
	}

	fn platform() -> Platform {
		if cfg!(target_os = "ios") {
			Platform::Ios
		} else if cfg!(target_os = "android") {
			Platform::Android
		} else if cfg!(target_os = "macos") {
			Platform::Macos
		} else if cfg!(target_os = "windows") {
			Platform::Windows
		} else {
			Platform::Linux
		}
	}
}

#[cfg(not(all(feature = "devtools", not(target_arch = "wasm32"))))]
fn configure(_: &str, _: &str) -> bool {
	false
}

#[cfg(not(all(feature = "devtools", not(target_arch = "wasm32"))))]
fn highlighted() -> Option<DevtoolsFrame> {
	None
}

#[cfg(not(all(feature = "devtools", not(target_arch = "wasm32"))))]
fn picking() -> bool {
	false
}
