//! The locales the browser prefers.

use crate::Locale;

/// The locales the browser prefers, best first: `navigator.languages`.
///
/// The browser is only asked in a browser build: with the `web` feature, on
/// `wasm32-unknown-unknown`. Everywhere else the list is empty, including on
/// the server, in tests, and in a native app's wasm guest, which builds
/// without `web`. Shared code can pass it to [`L10n::new`](crate::L10n::new)
/// on every platform and let the template locale stand in.
#[must_use]
pub fn browser_locales() -> Vec<Locale> {
	#[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
	{
		let Some(window) = web_sys::window() else {
			return Vec::new();
		};
		window
			.navigator()
			.languages()
			.iter()
			.filter_map(|value| value.as_string())
			.filter_map(|tag| Locale::new(&tag).ok())
			.collect()
	}
	#[cfg(not(all(feature = "web", target_arch = "wasm32", target_os = "unknown")))]
	{
		Vec::new()
	}
}

#[cfg(all(
	test,
	not(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))
))]
mod tests {
	#[test]
	fn there_is_no_browser_here() {
		assert!(super::browser_locales().is_empty());
	}
}
