//! The broker serves the public site, real documentation, console and API.
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use axum::Router;
use axum::http::HeaderValue;
use axum::http::header::CONTENT_SECURITY_POLICY;
use axum::response::Html;
use axum::response::Redirect;
use axum::routing::get;
use leptoskit::leptos::html;
use leptoskit::leptos::prelude::*;
use tower_http::services::ServeDir;
use tower_http::services::ServeFile;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::landing::Mode;

/// Assemble one same-origin site. Missing docs fail before accepting traffic.
pub fn router(api: Router, assets: &Path, docs: &Path, origin: &str) -> Result<Router> {
	for file in [
		"index.html",
		"console/index.html",
		"bootstrap.js",
		"pkg/leptoskit_app.js",
		"pkg/leptoskit_app_bg.wasm",
		SPLIT_LOADER,
		SPLIT_MANIFEST,
		"landing.css",
		"landing.mjs",
		"console.css",
		"rive/rive.js",
		"rive/rive.wasm",
		"rive/brand.riv",
	] {
		ensure!(
			assets.join(file).is_file(),
			"missing website asset {file}; build the complete site first"
		);
	}
	for file in split_chunks(assets)? {
		ensure!(
			assets.join(&file).is_file(),
			"missing website asset {file}; build the complete site first"
		);
	}
	for file in [
		"index.html",
		"guide/01-installation.html",
		"guide/29-hosted-deployments.html",
		"schema.json",
		"_leptoskit/theme.js",
		"_leptoskit/site.js",
		"_leptoskit/site.css",
		"_leptoskit/search.json",
		"theme/skribble.css",
		"assets/fonts/skribble/SkribbleNotoSketch-Regular.woff2",
		"assets/fonts/skribble/SkribbleMonoPlayful-Regular.woff2",
		"assets/fonts/skribble/SkribbleMonoPlayful-Bold.woff2",
		"_leptoskit/generated/card-1.svg",
		"_leptoskit/generated/brand-mint.svg",
	] {
		ensure!(
			docs.join(file).is_file(),
			"missing documentation {file}; package the docs before serving"
		);
	}
	let landing: Arc<str> = landing_html(origin, Mode::Hosted).into();
	let console = ServeFile::new(assets.join("console/index.html"));
	Ok(api
		.route(
			"/",
			get(move || {
				let html = Arc::clone(&landing);
				async move { Html(html.to_string()) }
			}),
		)
		.route(
			"/console",
			get(|| async { Redirect::permanent("/console/") }),
		)
		.route_service("/console/", console)
		.route("/docs", get(|| async { Redirect::permanent("/docs/") }))
		.nest_service("/docs/", ServeDir::new(docs))
		.fallback_service(ServeDir::new(assets))
		.layer(SetResponseHeaderLayer::if_not_present(
			CONTENT_SECURITY_POLICY,
			HeaderValue::from_static(
				"default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' \
				 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; object-src 'none'",
			),
		)))
}

/// The chunk loader `leptoskit build` writes beside a split client. The
/// console loads as a chunk of its own, so it cannot start without it.
const SPLIT_LOADER: &str = "pkg/__wasm_split.______________________.js";

/// The split client's manifest: each lazy entry point and the chunk files,
/// by stem, that it loads.
const SPLIT_MANIFEST: &str = "pkg/__wasm_split_manifest.json";

/// Every chunk file the split manifest in `assets` names, relative to
/// `assets`.
fn split_chunks(assets: &Path) -> Result<Vec<String>> {
	let manifest = std::fs::read(assets.join(SPLIT_MANIFEST))
		.with_context(|| format!("could not read {SPLIT_MANIFEST}"))?;
	let manifest: BTreeMap<String, Vec<String>> =
		serde_json::from_slice(&manifest).with_context(|| format!("invalid {SPLIT_MANIFEST}"))?;
	Ok(manifest
		.into_values()
		.flatten()
		.map(|stem| format!("pkg/{stem}.wasm"))
		.collect())
}

/// The ASCII subsets of the book's fonts that the landing page draws with;
/// `landing.css` names the same files in its `@font-face` rules.
pub const LANDING_FONTS: [&str; 2] = [
	"/docs/assets/fonts/skribble/SkribbleNotoSketch-Latin.woff2",
	"/docs/assets/fonts/skribble/SkribbleMonoPlayful-Latin.woff2",
];

/// Server-rendered builder components remain readable without JavaScript.
/// The hosted broker renders the pilot page; the public-only packager renders
/// [`Mode::Public`] so the static artifact offers no console or account
/// action its absent backend could not honor.
pub fn landing_html(origin: &str, mode: Mode) -> String {
	let owner = Owner::new();
	owner.with(|| {
		let title = "LeptosKit — One tree. Every screen. Your cloud.";
		let description = "Build interfaces, typed backends, databases and deployments in one \
		                   Rust codebase. Real native widgets. Your choice of cloud.";
		let page = html::html().lang("en").child((
			html::head().child((
				html::meta().charset("utf-8"),
				html::meta()
					.name("viewport")
					.content("width=device-width, initial-scale=1"),
				html::meta().name("description").content(description),
				html::meta().name("color-scheme").content("light dark"),
				html::script().src("/docs/_leptoskit/theme.js"),
				html::title().child(title),
				html::link()
					.rel("canonical")
					.href(format!("{}/", origin.trim_end_matches('/'))),
				html::link().rel("icon").href("/docs/assets/mark.svg"),
				// The page's lettering is small subsets of the book's fonts,
				// fetched beside the stylesheets so text draws in its own
				// hand on the first paint.
				LANDING_FONTS.map(|font| {
					html::link()
						.rel("preload")
						.href(font)
						.r#as("font")
						.r#type("font/woff2")
						.crossorigin("")
				}),
				html::link()
					.rel("stylesheet")
					.href("/docs/theme/skribble.css"),
				html::link().rel("stylesheet").href("/landing.css"),
				html::script().r#type("module").src("/landing.mjs"),
			)),
			html::body().child((
				leptoskit::web::render(crate::landing::screen(mode)),
				html::noscript().child(
					"The documentation and links work without JavaScript. Enable JavaScript to \
					 try the interactive component example.",
				),
			)),
		));
		format!("<!doctype html>{}", page.to_html())
	})
}
