//! The locales the browser prefers.

use crate::Locale;

/// The locales the browser prefers, best first: `navigator.languages`.
///
/// Empty outside a browser, including on the server and in a native wasm
/// host, so shared code can pass it to [`L10n::new`](crate::L10n::new)
/// everywhere and let the template locale stand in.
#[must_use]
pub fn browser_locales() -> Vec<Locale> {
	#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
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
	#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
	{
		Vec::new()
	}
}

#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod tests {
	#[test]
	fn there_is_no_browser_here() {
		assert!(super::browser_locales().is_empty());
	}
}
