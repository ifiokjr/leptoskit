//! The leptoskit documentation site: the book in `docs/src`, built with
//! `leptoskit_docs`, plus the handful of components only these docs use.

use std::path::Path;
use std::path::PathBuf;

use leptos::prelude::*;
use leptoskit_docs::Components;
use leptoskit_docs::PropError;
use leptoskit_docs::Props;
use leptoskit_docs::Server;
use leptoskit_docs::Site;

/// Where the site is published.
pub const URL: &str = "https://ifiokjr.github.io/leptoskit/";

/// The repository the docs live in.
pub const REPOSITORY: &str = "https://github.com/ifiokjr/leptoskit";

/// The book: `docs/src`, beside this crate.
pub fn book() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("../src")
}

/// The leptoskit site for the book in `root`.
pub fn site(root: impl Into<PathBuf>) -> Site {
	Site::new(root)
		.title("leptoskit")
		.description(
			"One leptos tree for the web and real native widgets on every platform, updated over \
			 the air.",
		)
		.url(URL)
		.repository(REPOSITORY)
		.edit_url(format!("{REPOSITORY}/edit/main/docs/src/{{path}}"))
		.logo("assets/logo-light.svg", "assets/logo-dark.svg")
		.favicon("assets/mark.svg")
		.social_image("assets/social.png")
		.stylesheet("theme/leptoskit.css")
		.components(components())
}

/// The built-in components and the ones only these docs use:
///
/// - `<Hero start="…">` opens the home page: the mark, whose mint piece lands
///   as the page loads, beside the Markdown it wraps and a call to action.
/// - `<Gallery>` holding `<Shot src="…" alt="…" title="…" wide>`: rendered
///   screens with captions.
/// - `<Swatches>` holding `<Swatch name="…" color="#…" />`: the brand colours.
/// - `<Logos>` holding `<Logo src="…" label="…" ink />`: logo files on paper or
///   ink.
pub fn components() -> Components {
	Components::new()
		.with("Hero", |props, children| {
			let start = props
				.url("start")
				.ok_or_else(|| PropError::new("<Hero> needs a `start` link"))?;
			Ok(view! { <Hero start>{children()}</Hero> }.into_any())
		})
		.with("Gallery", |_props, children| {
			Ok(view! { <div class="lk-gallery">{children()}</div> }.into_any())
		})
		.with("Shot", shot)
		.with("Swatches", |_props, children| {
			Ok(view! { <div class="lk-swatches">{children()}</div> }.into_any())
		})
		.with("Swatch", |props, _children| {
			let name = props.require("name")?.to_owned();
			let color = props.require("color")?.to_owned();
			Ok(view! { <Swatch name color /> }.into_any())
		})
		.with("Logos", |_props, children| {
			Ok(view! { <div class="lk-logos">{children()}</div> }.into_any())
		})
		.with("Logo", |props, _children| {
			let src = props
				.url("src")
				.ok_or_else(|| PropError::new("<Logo> needs a `src` image"))?;
			let label = props.require("label")?.to_owned();
			let ink = props.parse("ink")?.unwrap_or(false);
			Ok(view! { <Logo src label ink /> }.into_any())
		})
}

#[component]
fn Hero(start: String, children: Children) -> impl IntoView {
	view! {
		<section class="lk-hero">
			<div class="lk-hero-mark" aria-hidden="true">
				<span class="lk-box lk-box-tall" />
				<span class="lk-box lk-box-square" />
				<span class="lk-box lk-box-slot" />
				<span class="lk-box lk-box-ota" />
			</div>
			<div class="lk-hero-copy">
				{children()}
				<p class="lk-hero-actions">
					<a class="lk-button lk-button-primary" href=start>"Get started"</a>
					<a class="lk-button" href=REPOSITORY>"Source on GitHub"</a>
				</p>
			</div>
		</section>
	}
}

fn shot(props: &Props<'_>, children: Children) -> Result<AnyView, PropError> {
	let src = props
		.url("src")
		.ok_or_else(|| PropError::new("<Shot> needs a `src` image"))?;
	let alt = props.require("alt")?.to_owned();
	let title = props.require("title")?.to_owned();
	let wide = props.parse("wide")?.unwrap_or(false);
	let (width, height) = if wide {
		("1200", "800")
	} else {
		("390", "844")
	};

	Ok(view! {
		<figure class=if wide {
			"lk-shot lk-shot-wide"
		} else {
			"lk-shot"
		}>
			<img src=src alt=alt loading="lazy" width=width height=height />
			<figcaption>
				<strong>{title}</strong>
				{children()}
			</figcaption>
		</figure>
	}
	.into_any())
}

#[component]
fn Swatch(name: String, color: String) -> impl IntoView {
	view! {
		<div class="lk-swatch">
			<span class="lk-swatch-chip" style=format!("background: {color}") />
			<strong>{name}</strong>
			<code>{color.clone()}</code>
		</div>
	}
}

#[component]
fn Logo(src: String, label: String, ink: bool) -> impl IntoView {
	view! {
		<figure class=if ink {
			"lk-logo-tile lk-on-ink"
		} else {
			"lk-logo-tile"
		}>
			<img src=src alt="" />
			<figcaption>{label}</figcaption>
		</figure>
	}
}

/// Build or serve the leptoskit documentation site.
#[derive(Debug, clap::Parser)]
#[command(name = "leptoskit_docs_site")]
pub enum Cli {
	/// Build the site.
	Build {
		/// Where to write the site. Defaults to `docs/dist`.
		#[arg(long)]
		out: Option<PathBuf>,
	},
	/// Build the site, serve it, and rebuild and reload on every edit.
	Serve {
		/// Where to write the site. Defaults to `docs/dist`.
		#[arg(long)]
		out: Option<PathBuf>,
		/// The port to listen on, on 127.0.0.1.
		#[arg(long, default_value_t = 4400)]
		port: u16,
	},
}

impl Cli {
	/// Runs the command. `build` returns `None` once the site is written;
	/// `serve` returns the running server, which the caller waits on.
	pub fn run(self) -> Result<Option<Server>, leptoskit_docs::Error> {
		let default_out = || book().join("../dist");

		match self {
			Self::Build { out } => {
				let out = out.unwrap_or_else(default_out);
				let build = site(book()).build(&out)?;
				eprintln!(
					"built {} pages and copied {} files into {}",
					build.pages,
					build.files,
					out.display()
				);
				Ok(None)
			}
			Self::Serve { out, port } => {
				let server =
					site(book()).serve(out.unwrap_or_else(default_out), ("127.0.0.1", port))?;
				eprintln!("serving the docs at http://{}/", server.address());
				Ok(Some(server))
			}
		}
	}
}
