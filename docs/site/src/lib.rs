//! The leptoskit documentation site: the book in `docs/src`, built with
//! `leptoskit_docs`, plus the handful of components only these docs use.

pub mod ink;
pub mod play;

use std::path::Path;
use std::path::PathBuf;

use leptos::prelude::*;
use leptoskit_docs::Components;
use leptoskit_docs::PropError;
use leptoskit_docs::Props;
use leptoskit_docs::Server;
use leptoskit_docs::Site;

/// Where the site is published: the public domain's documentation root.
pub const URL: &str = "https://leptoskit.dev/docs/";

/// The repository the docs live in.
pub const REPOSITORY: &str = "https://github.com/leptoskit/leptoskit";

/// The book: `docs/src`, beside this crate.
pub fn book() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("../src")
}

/// The leptoskit site for the book in `root`.
pub fn site(root: impl Into<PathBuf>) -> Site {
	let mut site = Site::new(root)
		.title("leptoskit")
		.description("One leptos tree for the web and real native widgets on every platform.")
		.url(URL)
		.repository(REPOSITORY)
		.edit_url(format!("{REPOSITORY}/edit/main/docs/src/{{path}}"))
		.logo("assets/logo-light.svg", "assets/logo-dark.svg")
		.favicon("assets/mark.svg")
		.social_image("assets/social.png")
		.stylesheet("theme/leptoskit.css")
		.stylesheet("theme/skribble.css")
		.stylesheet("theme/play.css")
		.script("theme/play.js")
		.external_fonts(false)
		.components(components());
	for (name, bytes) in ink::files() {
		site = site.generated_asset(name, bytes);
	}
	site
}

/// The built-in components and the ones only these docs use:
///
/// - `<Hero start="…">` opens the home page: the mark, whose mint piece lands
///   as the page loads, beside the Markdown it wraps and a call to action.
/// - `<Gallery>` holding `<Shot src="…" alt="…" title="…" wide>`: rendered
///   screens with captions.
/// - `<Play app="…" title="…" description="…" start="?…" width="…" height="…"
///   native="…">`: an example's real web build in a framed player, with back,
///   reload, and open-in-a-new-tab controls and, given `native`, a switch to
///   the AppKit render (see [`play`]). Works inside a `<Gallery>`.
/// - `<Swatches>` holding `<Swatch name="…" color="#…" />`: the brand colours.
/// - `<Logos>` holding `<Logo src="…" label="…" ink />`: logo files on paper or
///   ink.
pub fn components() -> Components {
	Components::new()
		.with("Hero", |props, children| {
			let start = props
				.url("start")
				.ok_or_else(|| PropError::new("<Hero> needs a `start` link"))??;
			Ok(view! { <Hero start>{children()}</Hero> }.into_any())
		})
		.with("Gallery", |_props, children| {
			Ok(view! { <div class="lk-gallery">{children()}</div> }.into_any())
		})
		.with("Shot", shot)
		.with("Play", player)
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
				.ok_or_else(|| PropError::new("<Logo> needs a `src` image"))??;
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
		.ok_or_else(|| PropError::new("<Shot> needs a `src` image"))??;
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

/// The phone viewport, in CSS pixels, that players use unless told
/// otherwise: the gallery's phone screens and Field are laid out for it.
const PHONE: (u32, u32) = (390, 844);

/// A player: the example's web build, loaded into a sandboxed frame by
/// `theme/play.js` as it nears the viewport, laid out at its own `width` and
/// `height` and scaled to the column, so it never shifts the page. Without
/// JavaScript, which the apps need anyway, the native render shows instead.
fn player(props: &Props<'_>, children: Children) -> Result<AnyView, PropError> {
	let name = props.require("app")?;
	let example = play::example(name).ok_or_else(|| {
		PropError::new(format!(
			"<Play app=\"{name}\"> is not an example the docs build; add it to `play::EXAMPLES`"
		))
	})?;
	let start = props.get("start").unwrap_or_default();
	if !(start.is_empty()
		|| start.starts_with('?')
			&& start[1..]
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || b"=&-_.".contains(&byte)))
	{
		return Err(PropError::new(format!(
			"`start=\"{start}\"` on <Play> must be a query string such as `?screen=chat`"
		)));
	}
	let title = props.require("title")?.to_owned();
	let description = props.require("description")?.to_owned();
	let width = props.parse::<u32>("width")?.unwrap_or(PHONE.0);
	let height = props.parse::<u32>("height")?.unwrap_or(PHONE.1);
	if width == 0 || height == 0 {
		return Err(PropError::new(
			"<Play> needs a `width` and `height` above zero",
		));
	}
	let native = props.url("native").transpose()?;
	let bar = player_bar(&title, native.is_some());

	let id = format!(
		"lk-play-{}",
		title
			.chars()
			.map(|character| {
				if character.is_ascii_alphanumeric() {
					character.to_ascii_lowercase()
				} else {
					'-'
				}
			})
			.collect::<String>()
	);
	let description_id = format!("{id}-description");
	let class = if width > height {
		"lk-play lk-play-wide"
	} else {
		"lk-play"
	};
	let source = format!("{}/{}/{start}", play::DIRECTORY, example.name);
	let native_image = native.clone().map(|source| {
		view! {
			<img
				class="lk-play-native"
				data-src=source
				alt=description.clone()
				width=width
				height=height
				hidden
			/>
		}
	});
	// The apps need JavaScript, so without it the native render stands in.
	let fallback = match native {
		Some(source) => {
			view! {
				<noscript>
					<img
						class="lk-play-native"
						src=source
						alt=description.clone()
						width=width
						height=height
					/>
				</noscript>
			}
			.into_any()
		}
		None => {
			view! {
				<noscript>
					<p class="lk-play-message">
						{format!("{title} plays here with JavaScript on.")}
					</p>
				</noscript>
			}
			.into_any()
		}
	};

	Ok(view! {
		<figure
			class=class
			id=id.clone()
			data-lk-play=""
			data-width=width
			data-height=height
			style=format!("--lk-play-width: {width}px")
		>
			<div class="lk-play-device">
				{bar}
				<div class="lk-play-stage" style=format!("aspect-ratio: {width} / {height}")>
					<div class="lk-play-skeleton" aria-hidden="true">
						<span />
						<span />
						<span />
						<span />
					</div>
					<iframe
						class="lk-play-frame"
						title=format!("{title}, playable")
						aria-describedby=description_id.clone()
						data-src=source
						sandbox=example.sandbox()
						referrerpolicy="no-referrer"
						width=width
						height=height
					/>
					{native_image}
					{fallback}
					<p class="lk-play-status" role="status" data-lk-play-status="" />
				</div>
			</div>
			<figcaption>
				<strong>{title.clone()}</strong>
				" "
				<span id=description_id>{description.clone()}</span>
				" "
				{children()}
			</figcaption>
		</figure>
	}
	.into_any())
}

/// A player's toolbar: back through the app's own history, what renders it
/// and, with a native render, the switch to it, then reload and open in a new
/// tab. `theme/play.js` enables the controls once it runs.
fn player_bar(title: &str, native: bool) -> impl IntoView {
	let switch = native.then(|| {
		view! {
			<span class="lk-play-switch" role="group" aria-label=format!("{title}: renderer")>
				<button type="button" data-lk-play-view="web" aria-pressed="true" disabled>
					"Web"
				</button>
				<button type="button" data-lk-play-view="native" aria-pressed="false" disabled>
					"AppKit"
				</button>
			</span>
		}
	});
	view! {
		<div class="lk-play-bar">
			<button
				type="button"
				class="lk-icon-button"
				data-lk-play-back=""
				aria-label=format!("Back in {title}")
				title="Back"
				disabled
			>
				<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
					<path d="M15 5l-7 7 7 7" />
				</svg>
			</button>
			<span class="lk-play-label" data-lk-play-label="">"Web build, in your browser"</span>
			{switch}
			<button
				type="button"
				class="lk-icon-button"
				data-lk-play-reload=""
				aria-label=format!("Reload {title}")
				title="Reload"
				disabled
			>
				<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
					<path d="M19 12a7 7 0 1 1-2.05-4.95M19 4v4h-4" />
				</svg>
			</button>
			<a
				class="lk-icon-button"
				data-lk-play-open=""
				target="_blank"
				rel="noopener"
				aria-label=format!("Open {title} in a new tab")
				title="Open in a new tab"
			>
				<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
					<path d="M14 5h5v5M19 5l-8 8M17 14v5H5V7h5" />
				</svg>
			</a>
		</div>
	}
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
