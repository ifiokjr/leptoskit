//! The examples these docs play: each one's real web build, served beside the
//! book under `play/<name>/` and framed on a page by `<Play>`.
//!
//! `leptoskit-site` builds every example in [`EXAMPLES`] with `leptoskit build
//! --platform web --release` after it builds the book, then prepares each
//! build's page with [`frame_document`]. The book alone (`docs:serve`) has no
//! builds, so its players say the example is not built and offer the native
//! render instead.

/// An example app the docs embed as a player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Example {
	/// The name `<Play app="…">` uses and the directory under `play/` its
	/// build is served from.
	pub name: &'static str,
	/// The example's directory, relative to the repository root, where
	/// `leptoskit build` runs.
	pub directory: &'static str,
	/// Whether the app changes its path as it navigates. Safari lets a framed
	/// page change its path only when the page keeps its origin, so these
	/// frames are sandboxed with `allow-same-origin` as well.
	pub routes: bool,
}

impl Example {
	/// The frame's `sandbox` flags: the fewest the app needs. Every example
	/// runs scripts; only a routed one keeps its origin, and none can
	/// navigate the docs page, open windows, or submit forms.
	pub fn sandbox(&self) -> &'static str {
		if self.routes {
			"allow-scripts allow-same-origin"
		} else {
			"allow-scripts"
		}
	}
}

/// Every example the docs play.
pub const EXAMPLES: [Example; 4] = [
	Example {
		name: "gallery",
		directory: "examples/gallery",
		routes: false,
	},
	Example {
		name: "observatory",
		directory: "examples/observatory",
		routes: false,
	},
	Example {
		name: "postcard",
		directory: "examples/postcard",
		routes: false,
	},
	Example {
		name: "field",
		directory: "examples/file_routes",
		routes: true,
	},
];

/// The example named `name`.
pub fn example(name: &str) -> Option<&'static Example> {
	EXAMPLES.iter().find(|example| example.name == name)
}

/// Where played builds live, relative to the book's output directory.
pub const DIRECTORY: &str = "play";

/// The script each played build loads before its app, relative to the
/// build's own directory: it tells the player when the app starts, fails, or
/// moves through its history, and goes back when the player's button asks.
pub const FRAME_SCRIPT: &str = "../../theme/play-frame.js";

/// The base every played example's `web/index.html` declares, so its own
/// files load relative to wherever it is served.
const ROOT_BASE: &str = r#"<base href="/" />"#;

/// A played build's `index.html`, ready to serve from `base` (such as
/// `/docs/play/field/`): its `<base href="/" />` points at `base`, and the
/// frame script loads right after it, before anything else on the page.
///
/// # Errors
///
/// Fails unless the page declares `<base href="/" />` exactly once: without
/// it, the app's files and routes would resolve against the site's root.
pub fn frame_document(index: &str, base: &str) -> Result<String, String> {
	if index.matches(ROOT_BASE).count() != 1 {
		return Err(format!(
			"a played example's page must declare `{ROOT_BASE}` once, so it can be served from \
			 {base}"
		));
	}
	let frame = format!(r#"<base href="{base}" /><script src="{FRAME_SCRIPT}"></script>"#);
	Ok(index.replacen(ROOT_BASE, &frame, 1))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn names_are_unique_paths() {
		for (index, example) in EXAMPLES.iter().enumerate() {
			assert!(
				example
					.name
					.bytes()
					.all(|byte| byte.is_ascii_lowercase() || byte == b'_'),
				"{} must be a plain directory name",
				example.name
			);
			assert!(
				EXAMPLES[index + 1..]
					.iter()
					.all(|other| other.name != example.name),
				"{} is listed twice",
				example.name
			);
			assert!(
				std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
					.join("../..")
					.join(example.directory)
					.join("web/index.html")
					.is_file(),
				"{} has a web page to play",
				example.name
			);
		}
	}

	#[test]
	fn every_played_page_can_move_to_its_frame() {
		let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
		for example in EXAMPLES {
			let index =
				std::fs::read_to_string(root.join(example.directory).join("web/index.html"))
					.unwrap();
			let base = format!("/docs/play/{}/", example.name);
			let framed = frame_document(&index, &base)
				.unwrap_or_else(|error| panic!("{}: {error}", example.name));
			assert!(framed.contains(&format!(r#"<base href="{base}" />"#)));
			assert!(
				!framed.contains("<script>") && !framed.contains("<script type=\"module\">"),
				"{} keeps its scripts external, within the site's policy",
				example.name
			);
			assert!(
				!framed.contains("\"/pkg/") && !framed.contains("\"/assets/"),
				"{} loads its files relative to its base",
				example.name
			);
			let first_script = framed.find("<script").unwrap();
			assert!(
				framed[first_script..].starts_with(&format!(r#"<script src="{FRAME_SCRIPT}">"#)),
				"{} loads the frame script before its app",
				example.name
			);
		}
	}

	#[test]
	fn a_page_without_its_root_base_is_refused() {
		assert!(frame_document("<head></head>", "/docs/play/x/").is_err());
		let twice = format!("{ROOT_BASE}{ROOT_BASE}");
		assert!(frame_document(&twice, "/docs/play/x/").is_err());
		assert_eq!(
			frame_document(&format!("<head>{ROOT_BASE}</head>"), "/p/").unwrap(),
			format!(r#"<head><base href="/p/" /><script src="{FRAME_SCRIPT}"></script></head>"#)
		);
	}

	#[test]
	fn only_routed_examples_keep_their_origin() {
		assert_eq!(example("gallery").unwrap().sandbox(), "allow-scripts");
		assert_eq!(
			example("field").unwrap().sandbox(),
			"allow-scripts allow-same-origin"
		);
		assert!(example("huddle").is_none());
	}
}
