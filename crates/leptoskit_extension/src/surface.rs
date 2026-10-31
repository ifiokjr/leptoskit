//! The surfaces an extension shows, and mounting one.

/// A page surface the extension package declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PageName {
	/// The toolbar button's popup.
	Popup,
	/// The side panel, or Firefox's sidebar.
	SidePanel,
	/// The options page.
	Options,
	/// The new tab page.
	NewTab,
}

impl PageName {
	/// The surface's name in the package, such as `side_panel`.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Popup => "popup",
			Self::SidePanel => "side_panel",
			Self::Options => "options",
			Self::NewTab => "new_tab",
		}
	}
}

/// Where a screen is being opened, and the route it opens at.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Surface {
	/// One of the extension's own pages.
	Page {
		/// Which page.
		name: PageName,
		/// The configured route.
		route: String,
	},
	/// The frame a content script shows on another site's page. Use
	/// [`page::connect`](crate::page::connect) to reach that page.
	Content {
		/// The content script's configured name.
		name: String,
		/// The configured route.
		route: String,
	},
}

impl Surface {
	/// Reads the surface the generated page names: `popup`, `side_panel`,
	/// `options`, `new_tab`, or `content:<name>`.
	#[must_use]
	pub fn parse(surface: &str, route: &str) -> Option<Self> {
		let route = route.to_owned();
		if let Some(name) = surface.strip_prefix("content:") {
			return Some(Self::Content {
				name: name.to_owned(),
				route,
			});
		}
		let name = match surface {
			"popup" => PageName::Popup,
			"side_panel" => PageName::SidePanel,
			"options" => PageName::Options,
			"new_tab" => PageName::NewTab,
			_ => return None,
		};
		Some(Self::Page { name, route })
	}

	/// The route the surface opens at.
	#[must_use]
	pub fn route(&self) -> &str {
		match self {
			Self::Page { route, .. } | Self::Content { route, .. } => route,
		}
	}
}

/// Mounts `main`'s screen for the surface the generated page names.
#[cfg(target_arch = "wasm32")]
pub fn start<R>(surface: &str, route: &str, main: fn(Surface) -> R)
where
	R: Fn() -> leptoskit_native::Element + Send + Sync + 'static,
{
	let Some(surface) = Surface::parse(surface, route) else {
		leptos::logging::error!("leptoskit: the page named an unknown surface `{surface}`");
		return;
	};
	let render = main(surface);
	// The inner closure is the view: leptos runs it again whenever a signal
	// it read changes.
	leptos::mount::mount_to_body(move || move || leptoskit_native::web::render(render()));
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn surfaces_parse() {
		assert_eq!(
			Surface::parse("side_panel", "/panel"),
			Some(Surface::Page {
				name: PageName::SidePanel,
				route: "/panel".into()
			})
		);
		assert_eq!(
			Surface::parse("content:clipper", "/overlay"),
			Some(Surface::Content {
				name: "clipper".into(),
				route: "/overlay".into()
			})
		);
		assert_eq!(Surface::parse("devtools", "/"), None);
		assert_eq!(Surface::parse("popup", "/clip").unwrap().route(), "/clip");
	}
}
