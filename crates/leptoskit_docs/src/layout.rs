//! The page around every chapter: header, chapter navigation, table of
//! contents, pager, and search dialog.

use leptos::prelude::*;

use crate::Heading;
use crate::Site;
use crate::Summary;
use crate::SummaryItem;
use crate::render::to_static_html;
use crate::site::ASSETS;
use crate::site::output_path;
use crate::summary::Chapter;

const FONTS: &str = "https://fonts.googleapis.com/css2?family=Bricolage+Grotesque:opsz,wght@12..96,500;12..96,700;12..96,800&family=Instrument+Sans:ital,wght@0,400;0,500;0,600;1,400&family=JetBrains+Mono:wght@400;500&display=swap";

/// Applies a saved theme before the first paint, so a dark page never
/// flashes light.
const THEME: &str = r#"try{var t=localStorage.getItem("lk-theme");if(t==="light"||t==="dark")document.documentElement.dataset.theme=t}catch(e){}"#;

const MENU_ICON: &str = r#"<svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><path d="M4 7h16M4 12h16M4 17h10" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>"#;
const SEARCH_ICON: &str = r#"<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><circle cx="11" cy="11" r="6.5" fill="none" stroke="currentColor" stroke-width="2"/><path d="m16 16 4 4" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>"#;
const THEME_ICON: &str = r#"<svg class="lk-sun" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><circle cx="12" cy="12" r="4" fill="none" stroke="currentColor" stroke-width="2"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg><svg class="lk-moon" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5Z" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg>"#;
const REPOSITORY_ICON: &str = r#"<svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><path fill="currentColor" d="M12 2a10 10 0 0 0-3.16 19.49c.5.09.68-.22.68-.48v-1.7c-2.78.6-3.37-1.34-3.37-1.34-.45-1.16-1.11-1.47-1.11-1.47-.9-.62.07-.6.07-.6 1 .07 1.53 1.03 1.53 1.03.9 1.52 2.34 1.08 2.91.83.09-.65.35-1.09.63-1.34-2.22-.25-4.55-1.11-4.55-4.94 0-1.09.39-1.98 1.03-2.68-.1-.25-.45-1.27.1-2.64 0 0 .84-.27 2.75 1.02a9.6 9.6 0 0 1 5 0c1.91-1.3 2.75-1.02 2.75-1.02.55 1.37.2 2.39.1 2.64.64.7 1.03 1.59 1.03 2.68 0 3.84-2.34 4.68-4.57 4.93.36.31.68.92.68 1.85v2.75c0 .27.18.58.69.48A10 10 0 0 0 12 2Z"/></svg>"#;

/// A chapter before or after the current one.
pub(crate) struct Neighbour<'a> {
	pub(crate) title: &'a str,
	pub(crate) output: String,
}

pub(crate) struct Page<'a> {
	pub(crate) site: &'a Site,
	pub(crate) summary: &'a Summary,
	/// The chapter's source, for the edit link; `None` for generated pages.
	pub(crate) source: Option<&'a str>,
	pub(crate) output: &'a str,
	/// The relative path from this page to the site root.
	pub(crate) root: String,
	pub(crate) title: String,
	pub(crate) description: Option<String>,
	pub(crate) headings: &'a [Heading],
	pub(crate) previous: Option<&'a Neighbour<'a>>,
	pub(crate) next: Option<&'a Neighbour<'a>>,
	pub(crate) body: AnyView,
}

/// The whole HTML document for one page.
pub(crate) fn page(page: Page<'_>) -> String {
	let Page {
		site,
		summary,
		source,
		output,
		root,
		title,
		description,
		headings,
		previous,
		next,
		body,
	} = page;
	let edit = edit_link(site, source);
	let pages = pager(&root, previous, next);

	let html = view! {
		<html lang="en" data-root=root.clone() data-live-reload=site.live_reload.then_some("")>
			<head>{head(site, output, &root, &title, description)}</head>
			<body>
				<a class="lk-skip" href="#content">"Skip to content"</a>
				{header(site, &root)}
				<div class="lk-layout">
					<aside class="lk-sidebar" id="lk-sidebar">
						<nav aria-label="Chapters">{navigation(summary, output, &root)}</nav>
					</aside>
					<main class="lk-main" id="content">
						<article class="lk-article">{body}</article>
						<footer class="lk-page-footer">{edit} {pages}</footer>
					</main>
					{contents(headings)}
				</div>
				{search_dialog()}
			</body>
		</html>
	};

	format!("<!DOCTYPE html>{}", to_static_html(html))
}

/// The document's `<head>`: title, description, and the Open Graph tags link
/// previews read, then the site's icon, fonts, styles, and scripts.
fn head(
	site: &Site,
	output: &str,
	root: &str,
	title: &str,
	description: Option<String>,
) -> impl IntoView {
	let at = |path: &str| format!("{root}{path}");
	let is_home = output == "index.html";
	let document_title = if is_home || title == site.title {
		site.title.clone()
	} else {
		format!("{title} · {}", site.title)
	};

	let description = description
		.or_else(|| site.description.clone())
		.unwrap_or_default();
	let address = site.url.as_ref().map(|url| {
		let page = if is_home {
			String::new()
		} else {
			output.to_owned()
		};

		format!("{url}{page}")
	});
	let social_image = site
		.social_image
		.as_ref()
		.zip(site.url.as_ref())
		.map(|(image, url)| format!("{url}{image}"));

	view! {
		<meta charset="utf-8" />
		<meta name="viewport" content="width=device-width, initial-scale=1" />
		<title>{document_title.clone()}</title>
		<meta name="description" content=description.clone() />
		<meta name="color-scheme" content="light dark" />
		{open_graph("og:type", "website".to_owned())}
		{open_graph("og:site_name", site.title.clone())}
		{open_graph("og:title", document_title)}
		{open_graph("og:description", description)}
		{address.clone().map(|address| {
			view! {
				<link rel="canonical" href=address.clone() />
				{open_graph("og:url", address)}
			}
		})}
		{social_image.map(|image| {
			view! {
				{open_graph("og:image", image)}
				<meta name="twitter:card" content="summary_large_image" />
			}
		})}
		{site
			.favicon
			.as_ref()
			.map(|icon| view! { <link rel="icon" href=at(icon) /> })}
		<link rel="preconnect" href="https://fonts.googleapis.com" />
		<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin="" />
		<link rel="stylesheet" href=FONTS />
		<link rel="stylesheet" href=at(&format!("{ASSETS}/site.css")) />
		{site
			.stylesheets
			.iter()
			.map(|sheet| view! { <link rel="stylesheet" href=at(sheet) /> })
			.collect::<Vec<_>>()}
		<script inner_html=THEME />
		<script src=at(&format!("{ASSETS}/site.js")) defer=true />
	}
}

/// The bar across the top: the chapter menu, the site's brand, search, the
/// repository link, and the theme switch.
fn header(site: &Site, root: &str) -> impl IntoView {
	let at = |path: &str| format!("{root}{path}");

	let brand = match &site.logo {
		Some((light, dark)) => {
			view! {
				<img class="lk-logo lk-logo-light" src=at(light) alt=site.title.clone() />
				<img class="lk-logo lk-logo-dark" src=at(dark) alt=site.title.clone() />
			}
			.into_any()
		}
		None => view! { <span class="lk-brand-name">{site.title.clone()}</span> }.into_any(),
	};

	let repository = site.repository.as_ref().map(|repository| {
		view! {
			<a
				class="lk-icon-button"
				href=repository.clone()
				aria-label="Repository"
				inner_html=REPOSITORY_ICON
			/>
		}
	});

	view! {
		<header class="lk-header">
			<button
				class="lk-icon-button lk-menu"
				type="button"
				aria-label="Chapters"
				aria-controls="lk-sidebar"
				aria-expanded="false"
				data-lk-menu=""
				inner_html=MENU_ICON
			/>
			<a class="lk-brand" href=at("index.html")>{brand}</a>
			<div class="lk-header-actions">
				<button class="lk-search-button" type="button" data-lk-search-open="" hidden=true>
					<span class="lk-search-icon" inner_html=SEARCH_ICON />
					<span class="lk-search-hint">"Search"</span>
					<kbd>"/"</kbd>
				</button>
				{repository}
				<button
					class="lk-icon-button lk-theme"
					type="button"
					aria-label="Switch between light and dark"
					data-lk-theme=""
					hidden=true
					inner_html=THEME_ICON
				/>
			</div>
		</header>
	}
}

/// The link to the chapter's source in the site's repository, when the site
/// has an edit URL and the page has a source.
fn edit_link(site: &Site, source: Option<&str>) -> impl IntoView {
	site.edit_url
		.as_ref()
		.zip(source)
		.map(|(template, source)| {
			view! {
				<a class="lk-edit" href=template.replace("{path}", source)>"Edit this page"</a>
			}
		})
}

/// Links to the previous and next chapters, when there are any.
fn pager(
	root: &str,
	previous: Option<&Neighbour<'_>>,
	next: Option<&Neighbour<'_>>,
) -> impl IntoView {
	(previous.is_some() || next.is_some()).then(|| {
		let link = |neighbour: &Neighbour<'_>, class: &'static str, label: &'static str| {
			view! {
				<a class=class href=format!("{root}{}", neighbour.output)>
					<span class="lk-pager-label">{label}</span>
					<span class="lk-pager-title">{neighbour.title.to_owned()}</span>
				</a>
			}
		};
		view! {
			<nav class="lk-pager" aria-label="Pages">
				{previous.map(|neighbour| link(neighbour, "lk-pager-previous", "Previous"))}
				{next.map(|neighbour| link(neighbour, "lk-pager-next", "Next"))}
			</nav>
		}
	})
}

/// The search dialog `site.js` opens, empty until it loads the index.
fn search_dialog() -> impl IntoView {
	view! {
		<dialog class="lk-search" data-lk-search="" aria-label="Search">
			<div class="lk-search-field">
				<span class="lk-search-icon" inner_html=SEARCH_ICON />
				<input
					type="search"
					placeholder="Search the docs"
					aria-label="Search the docs"
					autocomplete="off"
					spellcheck="false"
				/>
				<kbd>"esc"</kbd>
			</div>
			<ol class="lk-search-results" role="listbox" aria-label="Results" />
		</dialog>
	}
}

/// The page GitHub Pages and most static hosts serve for a missing address.
/// It can be served at any depth, so it links from the site's absolute root.
pub(crate) fn not_found(site: &Site, summary: &Summary, root: String) -> String {
	let home = format!("{root}index.html");
	page(Page {
		site,
		summary,
		source: None,
		output: "404.html",
		root,
		title: "Page not found".to_owned(),
		description: None,
		headings: &[],
		previous: None,
		next: None,
		body: view! {
			<h1>"Page not found"</h1>
			<p>
				"Nothing lives at this address. It may have moved when the docs were reorganised."
			</p>
			<p>
				<a href=home>"Back to the start"</a>
			</p>
		}
		.into_any(),
	})
}

/// The chapter list, with the current page marked.
fn navigation(summary: &Summary, current: &str, root: &str) -> AnyView {
	let mut groups = Vec::new();
	let mut chapters = Vec::new();

	let flush = |chapters: &mut Vec<AnyView>, groups: &mut Vec<AnyView>| {
		if !chapters.is_empty() {
			groups
				.push(view! { <ul class="lk-nav-list">{std::mem::take(chapters)}</ul> }.into_any());
		}
	};

	for item in &summary.items {
		match item {
			SummaryItem::Chapter(chapter) => chapters.push(nav_chapter(chapter, current, root)),
			SummaryItem::Part(title) => {
				flush(&mut chapters, &mut groups);
				groups.push(view! { <p class="lk-nav-part">{title.clone()}</p> }.into_any());
			}
			SummaryItem::Separator => {
				flush(&mut chapters, &mut groups);
				groups.push(view! { <hr class="lk-nav-separator" /> }.into_any());
			}
		}
	}

	flush(&mut chapters, &mut groups);

	groups.into_any()
}

fn nav_chapter(chapter: &Chapter, current: &str, root: &str) -> AnyView {
	let link = match &chapter.path {
		Some(source) => {
			let output = output_path(source);
			let current = (output == current).then_some("page");
			view! {
				<a href=format!("{root}{output}") aria-current=current>{chapter.title.clone()}</a>
			}
			.into_any()
		}
		None => view! { <span class="lk-nav-draft">{chapter.title.clone()}</span> }.into_any(),
	};

	let children = (!chapter.children.is_empty()).then(|| {
		let children = chapter
			.children
			.iter()
			.map(|child| nav_chapter(child, current, root))
			.collect::<Vec<_>>();
		view! { <ul>{children}</ul> }
	});

	view! { <li>{link} {children}</li> }.into_any()
}

/// "On this page": the page's second- and third-level headings, when there
/// are enough to be worth listing.
fn contents(headings: &[Heading]) -> Option<AnyView> {
	let entries = headings
		.iter()
		.filter(|heading| matches!(heading.depth, 2 | 3))
		.collect::<Vec<_>>();

	(entries.len() >= 2).then(|| {
		let items = entries
			.into_iter()
			.map(|heading| {
				view! {
					<li class=format!("lk-toc-{}", heading.depth)>
						<a href=format!("#{}", heading.id)>{heading.text.clone()}</a>
					</li>
				}
			})
			.collect::<Vec<_>>();

		view! {
			<aside class="lk-toc">
				<nav aria-label="On this page">
					<p class="lk-toc-title">"On this page"</p>
					<ul>{items}</ul>
				</nav>
			</aside>
		}
		.into_any()
	})
}

/// An Open Graph `<meta property="…">`. `property` is not an attribute the
/// `view!` macro knows, so it is added by name.
fn open_graph(property: &'static str, content: String) -> impl IntoView {
	leptos::html::meta().content(content).add_any_attr(
		leptos::tachys::html::attribute::custom::custom_attribute("property", property),
	)
}
