//! The interface the hub serves at `/`.
//!
//! The interface is the leptoskit app in this repository's `devtools/`,
//! built for the web. The hub looks for a build in this order:
//!
//! 1. `LEPTOSKIT_DEVTOOLS_UI=<dir>`: a local build, for working on the
//!    interface or on a CLI built from a checkout.
//! 2. The archive downloaded for exactly this CLI's version, in `<user
//!    cache>/leptoskit/devtools/<version>/`.
//! 3. Otherwise the plain page built into the CLI, with a note on how to get
//!    the full interface.
//!
//! The lookup runs on every request, so an interface downloaded while the hub
//! runs appears on the next reload.

use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use axum::body::Body;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::Response;

/// The environment variable naming a local build of the interface.
pub const UI_ENV: &str = "LEPTOSKIT_DEVTOOLS_UI";

/// Who may frame the hub's pages: the hub itself and the browser `DevTools`
/// panel. Firefox gives every install a random `moz-extension:` ID.
pub const FRAME_ANCESTORS: &str = "frame-ancestors 'self' chrome-extension: moz-extension:";

/// The plain page served when no interface build is available.
const FALLBACK: &str = include_str!("page.html");

/// The marker in [`FALLBACK`] replaced with why the full interface is missing.
const NOTICE: &str = "<!-- notice -->";

/// Where the hub looks for the interface.
#[derive(Clone, Debug, Default)]
pub struct Sources {
	/// A local build named by [`UI_ENV`].
	pub local: Option<PathBuf>,
	/// The downloaded build for this CLI's version.
	pub cache: Option<PathBuf>,
}

impl Sources {
	/// The sources from the environment and the user's cache directory.
	#[must_use]
	pub fn from_env() -> Self {
		Self {
			local: std::env::var_os(UI_ENV)
				.filter(|value| !value.is_empty())
				.map(PathBuf::from),
			cache: version_dir(env!("CARGO_PKG_VERSION")),
		}
	}

	/// The first source holding a build, and what it is.
	#[must_use]
	pub fn resolve(&self) -> Interface {
		if let Some(local) = &self.local {
			return if has_build(local) {
				Interface::Local(local.clone())
			} else {
				Interface::Missing(format!(
					"{UI_ENV} is set to {}, which has no index.html. Build the interface with \
					 <code>devenv shell devtools:build</code>, or unset {UI_ENV}.",
					escape(&local.display().to_string())
				))
			};
		}

		match &self.cache {
			Some(cache) if has_build(cache) => Interface::Downloaded(cache.clone()),
			_ => Interface::Missing(missing_notice()),
		}
	}
}

/// The interface the hub serves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Interface {
	/// A local build from [`UI_ENV`].
	Local(PathBuf),
	/// The build downloaded for this CLI's version.
	Downloaded(PathBuf),
	/// No build: the plain page, with this explanation as HTML.
	Missing(String),
}

impl Interface {
	/// A short name for `GET /api/hub`.
	#[must_use]
	pub fn kind(&self) -> &'static str {
		match self {
			Self::Local(_) => "local",
			Self::Downloaded(_) => "downloaded",
			Self::Missing(_) => "missing",
		}
	}

	/// The response for a `GET` of `path`.
	#[must_use]
	pub fn respond(&self, path: &str) -> Response {
		match self {
			Self::Local(dir) | Self::Downloaded(dir) => serve_file(dir, path),
			Self::Missing(notice) if is_index(path) => {
				page(
					"text/html; charset=utf-8",
					FALLBACK.replace(NOTICE, &format!("<p id=\"notice\">{notice}</p>")),
				)
			}
			Self::Missing(_) => StatusCode::NOT_FOUND.into_response(),
		}
	}
}

/// Where the interface for `version` is downloaded: `$XDG_CACHE_HOME`, or
/// `~/.cache`, under `leptoskit/devtools/<version>`.
#[must_use]
pub fn version_dir(version: &str) -> Option<PathBuf> {
	let base = std::env::var_os("XDG_CACHE_HOME")
		.map(PathBuf::from)
		.filter(|dir| dir.is_absolute())
		.or_else(|| {
			std::env::var_os("HOME")
				.or_else(|| std::env::var_os("USERPROFILE"))
				.map(|home| PathBuf::from(home).join(".cache"))
		})?;
	Some(base.join("leptoskit").join("devtools").join(version))
}

/// Whether `dir` holds a build of the interface.
#[must_use]
pub fn has_build(dir: &Path) -> bool {
	dir.join("index.html").is_file()
}

fn is_index(path: &str) -> bool {
	matches!(path.trim_start_matches('/'), "" | "index.html")
}

fn serve_file(dir: &Path, path: &str) -> Response {
	let relative = path.trim_start_matches('/');
	let relative = if relative.is_empty() {
		"index.html"
	} else {
		relative
	};
	let Some(file) = contained(dir, relative) else {
		return StatusCode::NOT_FOUND.into_response();
	};

	match std::fs::read(&file) {
		Ok(bytes) => page(content_type(&file), bytes),
		Err(_) => StatusCode::NOT_FOUND.into_response(),
	}
}

/// `relative` inside `dir`, refusing anything that could leave it.
fn contained(dir: &Path, relative: &str) -> Option<PathBuf> {
	let relative = Path::new(relative);
	relative
		.components()
		.all(|component| matches!(component, Component::Normal(_)))
		.then(|| dir.join(relative))
}

fn page(content_type: &'static str, body: impl Into<Body>) -> Response {
	let mut response = Response::new(body.into());
	let headers = response.headers_mut();
	headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
	headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
	headers.insert(
		header::CONTENT_SECURITY_POLICY,
		HeaderValue::from_static(FRAME_ANCESTORS),
	);
	headers.insert(
		header::X_CONTENT_TYPE_OPTIONS,
		HeaderValue::from_static("nosniff"),
	);
	response
}

fn content_type(file: &Path) -> &'static str {
	match file.extension().and_then(|extension| extension.to_str()) {
		Some("html") => "text/html; charset=utf-8",
		Some("js" | "mjs") => "text/javascript; charset=utf-8",
		Some("wasm") => "application/wasm",
		Some("css") => "text/css; charset=utf-8",
		Some("json" | "map") => "application/json",
		Some("svg") => "image/svg+xml",
		Some("png") => "image/png",
		Some("ico") => "image/x-icon",
		Some("txt") => "text/plain; charset=utf-8",
		_ => "application/octet-stream",
	}
}

/// Why the full interface is missing when nothing names a local build.
fn missing_notice() -> String {
	let version = env!("CARGO_PKG_VERSION");

	if super::fetch::expected_hash().is_some() {
		format!(
			"This is the hub's plain page: the full interface for leptoskit {version} isn't \
			 downloaded yet. <code>leptoskit dev</code> downloads it the first time it runs; if \
			 that failed, for example offline, run <code>leptoskit devtools fetch</code>, then \
			 reload."
		)
	} else {
		format!(
			"This is the hub's plain page. This leptoskit {version} was built from source, so it \
			 can't download the full interface. In a checkout of leptoskit, build it with \
			 <code>devenv shell devtools:build</code> and set {UI_ENV} to the output; a released \
			 leptoskit downloads it."
		)
	}
}

/// Escapes text for HTML.
pub(crate) fn escape(text: &str) -> String {
	text.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
		.replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
	use axum::body::to_bytes;

	use super::*;

	fn build() -> tempfile::TempDir {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("index.html"), "<html>ui</html>").unwrap();
		std::fs::create_dir(dir.path().join("pkg")).unwrap();
		std::fs::write(dir.path().join("pkg/app_bg.wasm"), b"\0asm").unwrap();
		std::fs::write(dir.path().join("pkg/app.js"), "export {}").unwrap();
		dir
	}

	async fn text(response: Response) -> String {
		let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
		String::from_utf8(bytes.to_vec()).unwrap()
	}

	#[test]
	fn a_local_build_wins_over_the_cache() {
		let local = build();
		let cache = build();
		let sources = Sources {
			local: Some(local.path().to_owned()),
			cache: Some(cache.path().to_owned()),
		};
		assert_eq!(sources.resolve(), Interface::Local(local.path().to_owned()));
		let sources = Sources {
			local: None,
			cache: Some(cache.path().to_owned()),
		};
		assert_eq!(
			sources.resolve(),
			Interface::Downloaded(cache.path().to_owned())
		);
	}

	#[test]
	fn a_local_setting_without_a_build_explains_itself() {
		let empty = tempfile::tempdir().unwrap();
		let sources = Sources {
			local: Some(empty.path().to_owned()),
			cache: None,
		};
		let Interface::Missing(notice) = sources.resolve() else {
			panic!("an empty directory is not a build");
		};
		assert!(notice.contains(UI_ENV));
		assert!(notice.contains("devtools:build"));
	}

	#[test]
	fn no_build_falls_back_to_the_plain_page() {
		let sources = Sources {
			local: None,
			cache: Some(PathBuf::from("/nonexistent/leptoskit/devtools")),
		};
		assert_eq!(sources.resolve().kind(), "missing");
	}

	#[tokio::test]
	async fn files_are_served_with_their_types_and_the_frame_policy() {
		let dir = build();
		let interface = Interface::Local(dir.path().to_owned());

		let response = interface.respond("/");
		assert_eq!(response.status(), StatusCode::OK);
		assert_eq!(
			response.headers()[header::CONTENT_TYPE],
			"text/html; charset=utf-8"
		);
		assert_eq!(
			response.headers()[header::CONTENT_SECURITY_POLICY],
			FRAME_ANCESTORS
		);
		assert_eq!(text(response).await, "<html>ui</html>");

		let response = interface.respond("/pkg/app_bg.wasm");
		assert_eq!(response.headers()[header::CONTENT_TYPE], "application/wasm");
		let response = interface.respond("/pkg/app.js");
		assert_eq!(
			response.headers()[header::CONTENT_TYPE],
			"text/javascript; charset=utf-8"
		);
		assert_eq!(
			interface.respond("/pkg/missing.js").status(),
			StatusCode::NOT_FOUND
		);
	}

	#[test]
	fn paths_cannot_leave_the_build() {
		let dir = build();
		let interface = Interface::Local(dir.path().join("pkg"));
		assert_eq!(
			interface.respond("/../index.html").status(),
			StatusCode::NOT_FOUND
		);
		assert_eq!(
			interface.respond("//etc/passwd").status(),
			StatusCode::NOT_FOUND
		);
		assert!(contained(dir.path(), "a/../../b").is_none());
		assert!(contained(dir.path(), "a/./b").is_some());
	}

	#[tokio::test]
	async fn the_fallback_page_carries_the_notice() {
		let interface = Interface::Missing("run <code>leptoskit devtools fetch</code>".into());
		let response = interface.respond("/");
		assert_eq!(
			response.headers()[header::CONTENT_SECURITY_POLICY],
			FRAME_ANCESTORS
		);
		let body = text(response).await;
		assert!(body.contains("<p id=\"notice\">run <code>leptoskit devtools fetch</code></p>"));
		assert!(body.contains("/api/stream"));
		assert_eq!(
			interface.respond("/pkg/app.js").status(),
			StatusCode::NOT_FOUND
		);
	}
}
