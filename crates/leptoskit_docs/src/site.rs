use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use serde::Serialize;

use crate::Chapter;
use crate::Components;
use crate::Diagnostic;
use crate::Document;
use crate::Error;
use crate::Rendered;
use crate::Renderer;
use crate::Summary;
use crate::layout;

/// The file in an output directory that marks it as ours, so a rebuild may
/// delete what is there.
const MARKER: &str = ".leptoskit-docs";

/// Where the generated stylesheet, script, and search index live in the
/// output, out of the way of the book's own files.
pub(crate) const ASSETS: &str = "_leptoskit";

/// A static documentation site built from a book of Markdown documents.
///
/// The book is a directory with an mdBook-style `SUMMARY.md` listing its
/// chapters (see [`Summary`]). Every chapter becomes a page beside its source:
/// `guide/install.md` builds `guide/install.html`, and a `readme.md` or
/// `index.md` builds the directory's `index.html`. Every other file in the
/// directory, such as images, is copied as is.
///
/// ```no_run
/// use leptoskit_docs::Site;
///
/// let build = Site::new("docs/src")
/// 	.title("leptoskit")
/// 	.repository("https://github.com/ifiokjr/leptoskit")
/// 	.build("docs/dist")?;
/// println!("built {} pages", build.pages);
/// # Ok::<(), leptoskit_docs::Error>(())
/// ```
///
/// A build fails, listing every problem, when a document does not parse,
/// uses an unknown component or prop, or links to a page, heading, or file
/// that does not exist.
#[derive(Debug, Clone)]
pub struct Site {
	pub(crate) root: PathBuf,
	pub(crate) title: String,
	pub(crate) description: Option<String>,
	pub(crate) url: Option<String>,
	pub(crate) repository: Option<String>,
	pub(crate) edit_url: Option<String>,
	pub(crate) logo: Option<(String, String)>,
	pub(crate) favicon: Option<String>,
	pub(crate) social_image: Option<String>,
	pub(crate) stylesheets: Vec<String>,
	pub(crate) components: Components,
	pub(crate) live_reload: bool,
}

/// What a [`Site::build`] wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Build {
	/// The pages rendered from chapters.
	pub pages: usize,
	/// The files copied from the book as they are.
	pub files: usize,
}

impl Site {
	/// A site for the book in `root`, the directory holding `SUMMARY.md`, with
	/// the built-in [`Components`].
	pub fn new(root: impl Into<PathBuf>) -> Self {
		Self {
			root: root.into(),
			title: "Documentation".to_owned(),
			description: None,
			url: None,
			repository: None,
			edit_url: None,
			logo: None,
			favicon: None,
			social_image: None,
			stylesheets: Vec::new(),
			components: Components::new(),
			live_reload: false,
		}
	}

	/// The site's name, shown in the header and every page title.
	#[must_use]
	pub fn title(mut self, title: impl Into<String>) -> Self {
		self.title = title.into();
		self
	}

	/// A one-line description, used for pages without a paragraph of their
	/// own.
	#[must_use]
	pub fn description(mut self, description: impl Into<String>) -> Self {
		self.description = Some(description.into());
		self
	}

	/// The address the site is published at, such as
	/// `https://example.github.io/project/`, for canonical links, link
	/// previews, and the 404 page.
	#[must_use]
	pub fn url(mut self, url: impl Into<String>) -> Self {
		let mut url = url.into();
		if !url.ends_with('/') {
			url.push('/');
		}
		self.url = Some(url);
		self
	}

	/// The project's repository, linked from the header.
	#[must_use]
	pub fn repository(mut self, repository: impl Into<String>) -> Self {
		self.repository = Some(repository.into());
		self
	}

	/// An "Edit this page" link for every page, where `{path}` is replaced by
	/// the chapter's source path, such as
	/// `https://github.com/me/project/edit/main/docs/{path}`.
	#[must_use]
	pub fn edit_url(mut self, template: impl Into<String>) -> Self {
		self.edit_url = Some(template.into());
		self
	}

	/// Logo images in the book for light and dark themes, shown in place of
	/// the title in the header.
	#[must_use]
	pub fn logo(mut self, light: impl Into<String>, dark: impl Into<String>) -> Self {
		self.logo = Some((light.into(), dark.into()));
		self
	}

	/// An icon in the book, such as `assets/icon.svg`.
	#[must_use]
	pub fn favicon(mut self, path: impl Into<String>) -> Self {
		self.favicon = Some(path.into());
		self
	}

	/// An image in the book for link previews. Needs [`Site::url`], since
	/// previews require an absolute address.
	#[must_use]
	pub fn social_image(mut self, path: impl Into<String>) -> Self {
		self.social_image = Some(path.into());
		self
	}

	/// A stylesheet in the book, loaded after the built-in one, to restyle the
	/// site or style your own components.
	#[must_use]
	pub fn stylesheet(mut self, path: impl Into<String>) -> Self {
		self.stylesheets.push(path.into());
		self
	}

	/// The components documents can use, replacing the built-in set. Start
	/// from [`Components::new`] to keep the built-in components.
	#[must_use]
	pub fn components(mut self, components: Components) -> Self {
		self.components = components;
		self
	}

	/// Builds the site into `out`, replacing an earlier build there.
	///
	/// Nothing is written unless every page renders and every link resolves.
	/// `out` must be missing, empty, or an earlier build.
	pub fn build(&self, out: impl AsRef<Path>) -> Result<Build, Error> {
		let out = out.as_ref();
		let summary_path = self.root.join("SUMMARY.md");
		let summary_source =
			fs::read_to_string(&summary_path).map_err(Error::io("read", &summary_path))?;
		let summary = Summary::parse(&summary_source).map_err(|diagnostics| {
			Error::Docs(
				diagnostics
					.into_iter()
					.map(|diagnostic| diagnostic.in_file("SUMMARY.md"))
					.collect(),
			)
		})?;

		let chapters = summary.pages();
		let mut diagnostics = Vec::new();
		let mut pages = Vec::new();
		let mut seen = HashSet::new();

		for chapter in &chapters {
			let source = chapter.path.as_deref().expect("pages have paths");
			if !seen.insert(source) {
				diagnostics.push(
					Diagnostic::new(0, 0, format!("`{source}` is listed more than once"))
						.in_file("SUMMARY.md"),
				);
				continue;
			}

			match self.render_page(source) {
				Ok(rendered) => pages.push((*chapter, rendered)),
				Err(problems) => diagnostics.extend(problems),
			}
		}

		diagnostics.extend(self.check_links(&pages));
		if !diagnostics.is_empty() {
			return Err(Error::Docs(diagnostics));
		}

		prepare_output(out)?;

		let outline = pages
			.iter()
			.map(|(chapter, _)| {
				layout::Neighbour {
					title: &chapter.title,
					output: output_path(chapter.path.as_deref().expect("pages have paths")),
				}
			})
			.collect::<Vec<_>>();
		let page_count = pages.len();
		let mut search = Vec::new();

		for (index, (chapter, rendered)) in pages.into_iter().enumerate() {
			let source = chapter.path.as_deref().expect("pages have paths");
			let output = &outline[index].output;
			search.extend(search_entries(chapter, &rendered, output));

			let html = layout::page(layout::Page {
				site: self,
				summary: &summary,
				source: Some(source),
				output,
				root: root_prefix(output),
				title: rendered
					.title
					.clone()
					.unwrap_or_else(|| chapter.title.clone()),
				description: rendered.summary.clone(),
				headings: &rendered.headings,
				previous: index
					.checked_sub(1)
					.and_then(|previous| outline.get(previous)),
				next: outline.get(index + 1),
				body: rendered.view,
			});
			write(&out.join(output), html.as_bytes())?;
		}

		let not_found_root = self
			.url
			.as_deref()
			.and_then(|url| url.split_once("://"))
			.and_then(|(_, rest)| rest.find('/').map(|slash| rest[slash..].to_owned()))
			.unwrap_or_else(|| "/".to_owned());
		write(
			&out.join("404.html"),
			layout::not_found(self, &summary, not_found_root).as_bytes(),
		)?;

		let files = self.copy_files(out)?;
		write(
			&out.join(ASSETS).join("site.css"),
			include_bytes!("../assets/site.css"),
		)?;
		write(
			&out.join(ASSETS).join("site.js"),
			include_bytes!("../assets/site.js"),
		)?;
		write(
			&out.join(ASSETS).join("search.json"),
			serde_json::to_string(&search)
				.expect("search entries serialize")
				.as_bytes(),
		)?;

		Ok(Build {
			pages: page_count,
			files,
		})
	}

	fn render_page(&self, source: &str) -> Result<Rendered, Vec<Diagnostic>> {
		let path = self.root.join(source);
		let text = fs::read_to_string(&path).map_err(|error| {
			vec![
				Diagnostic::new(
					0,
					0,
					format!("`{source}` is listed but cannot be read: {error}"),
				)
				.in_file("SUMMARY.md"),
			]
		})?;
		let document =
			Document::parse(&text).map_err(|diagnostic| vec![diagnostic.in_file(source)])?;

		Renderer::new(&self.components)
			.rewrite_urls(page_url)
			.render(&document)
			.map_err(|diagnostics| {
				diagnostics
					.into_iter()
					.map(|diagnostic| diagnostic.in_file(source))
					.collect()
			})
	}

	/// Checks every relative link and image: a `.md` target must be a page,
	/// an `#anchor` must be a heading or footnote on its page, and anything
	/// else must be a file in the book.
	fn check_links(&self, pages: &[(&Chapter, Rendered)]) -> Vec<Diagnostic> {
		let anchors: HashMap<&str, HashSet<&str>> = pages
			.iter()
			.map(|(chapter, rendered)| {
				(
					chapter.path.as_deref().expect("pages have paths"),
					rendered.anchors().collect(),
				)
			})
			.collect();
		let mut diagnostics = Vec::new();

		for (chapter, rendered) in pages {
			let source = chapter.path.as_deref().expect("pages have paths");
			for link in &rendered.links {
				let Some(problem) = self.link_problem(source, &link.url, &anchors) else {
					continue;
				};
				diagnostics.push(
					Diagnostic::new(
						link.line,
						link.column,
						format!("broken link `{}`: {problem}", link.url),
					)
					.in_file(source),
				);
			}
		}

		diagnostics
	}

	/// What is wrong with a link to `url` from the chapter `source`, if
	/// anything. `anchors` holds every page's linkable ids.
	fn link_problem(
		&self,
		source: &str,
		url: &str,
		anchors: &HashMap<&str, HashSet<&str>>,
	) -> Option<String> {
		match classify(url) {
			Target::External => None,
			Target::Local => {
				Some(
					"a local address only works on the reader's own machine; write it as inline \
					 code"
						.to_owned(),
				)
			}
			Target::Absolute => {
				Some(
					"absolute paths break when the site is served from a subdirectory; use a \
					 relative path"
						.to_owned(),
				)
			}
			Target::Relative { path, anchor } => {
				let target = if path.is_empty() {
					Some(source.to_owned())
				} else {
					resolve(source, path)
				};
				let Some(target) = target else {
					return Some("points outside the book".to_owned());
				};
				if markdown_stem(&target).is_none() {
					return (!self.root.join(&target).is_file())
						.then(|| format!("`{target}` does not exist in the book"));
				}
				match (anchors.get(target.as_str()), anchor) {
					(None, _) => Some(format!("`{target}` is not a chapter in SUMMARY.md")),
					(Some(ids), Some(anchor)) if !ids.contains(anchor) => {
						Some(format!("`{target}` has no heading with the id `{anchor}`"))
					}
					_ => None,
				}
			}
		}
	}

	/// Copies every file in the book that is not a chapter source.
	fn copy_files(&self, out: &Path) -> Result<usize, Error> {
		let out = fs::canonicalize(out).map_err(Error::io("read", out))?;
		let mut copied = 0;

		for entry in walkdir::WalkDir::new(&self.root)
			.into_iter()
			.filter_entry(|entry| {
				let hidden =
					entry.depth() > 0 && entry.file_name().to_string_lossy().starts_with('.');
				let inside_out = fs::canonicalize(entry.path()).is_ok_and(|path| path == out);
				!hidden && !inside_out
			}) {
			let entry = entry.map_err(|error| {
				let path = error.path().unwrap_or(&self.root).to_owned();
				Error::Io {
					action: "read",
					path,
					source: error.into(),
				}
			})?;

			if !entry.file_type().is_file()
				|| entry
					.path()
					.extension()
					.is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
			{
				continue;
			}

			let relative = entry
				.path()
				.strip_prefix(&self.root)
				.expect("walked inside the root");
			let target = out.join(relative);
			if let Some(parent) = target.parent() {
				fs::create_dir_all(parent).map_err(Error::io("create", parent))?;
			}
			fs::copy(entry.path(), &target).map_err(Error::io("copy", entry.path()))?;
			copied += 1;
		}

		Ok(copied)
	}
}

/// One search result: a section of a page.
#[derive(Debug, Serialize)]
struct SearchEntry {
	/// The chapter's title.
	page: String,
	/// The section's heading, when it is not the page's title.
	heading: Option<String>,
	/// The section's address, relative to the site root.
	url: String,
	text: String,
}

fn search_entries(chapter: &Chapter, rendered: &Rendered, output: &str) -> Vec<SearchEntry> {
	rendered
		.sections
		.iter()
		.filter(|section| !section.text.is_empty() || section.heading.is_some())
		.map(|section| {
			let heading = section
				.heading
				.clone()
				.filter(|heading| Some(heading) != rendered.title.as_ref());
			let url = match (&section.id, &heading) {
				(Some(id), Some(_)) => format!("{output}#{id}"),
				_ => output.to_owned(),
			};

			SearchEntry {
				page: chapter.title.clone(),
				heading,
				url,
				text: section.text.clone(),
			}
		})
		.collect()
}

enum Target<'a> {
	External,
	/// An `http(s)` address on the reader's own machine, such as a dev
	/// server's `http://localhost:3000`, which no published link can reach.
	Local,
	Absolute,
	Relative {
		path: &'a str,
		anchor: Option<&'a str>,
	},
}

fn classify(url: &str) -> Target<'_> {
	let scheme = url.split_once(':').is_some_and(|(scheme, _)| {
		scheme.starts_with(|first: char| first.is_ascii_alphabetic())
			&& scheme.chars().all(|character| {
				character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
			})
	});

	if is_local_address(url) {
		Target::Local
	} else if scheme || url.starts_with("//") {
		Target::External
	} else if url.starts_with('/') {
		Target::Absolute
	} else {
		let (path, anchor) = match url.split_once('#') {
			Some((path, anchor)) => (path, Some(anchor)),
			None => (url, None),
		};
		let path = path.split_once('?').map_or(path, |(path, _)| path);
		Target::Relative { path, anchor }
	}
}

/// Whether `url` is an `http(s)` address on the loopback interface or the
/// unspecified address: `localhost` and its subdomains, `127.0.0.0/8`,
/// `0.0.0.0`, and `[::1]`.
fn is_local_address(url: &str) -> bool {
	let Some((scheme, rest)) = url.split_once("://") else {
		return false;
	};
	if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
		return false;
	}

	let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
	let host_and_port = authority
		.rsplit_once('@')
		.map_or(authority, |(_, host)| host);
	let host = if let Some(bracketed) = host_and_port.strip_prefix('[') {
		bracketed
			.split_once(']')
			.map_or(bracketed, |(host, _)| host)
	} else {
		host_and_port
			.split_once(':')
			.map_or(host_and_port, |(host, _)| host)
	};
	let host = host.trim_end_matches('.').to_ascii_lowercase();

	host == "localhost"
		|| host.ends_with(".localhost")
		|| host == "0.0.0.0"
		|| host
			.parse::<std::net::IpAddr>()
			.is_ok_and(|ip| ip.is_loopback())
}

/// `path` relative to the page `source`, as a path from the book root, or
/// `None` when it climbs out of the book.
fn resolve(source: &str, path: &str) -> Option<String> {
	let mut parts: Vec<&str> = source.split('/').collect();
	parts.pop();

	for part in path.split('/') {
		match part {
			"" | "." => {}
			".." => {
				parts.pop()?;
			}
			part => parts.push(part),
		}
	}

	Some(parts.join("/"))
}

/// The page a chapter source builds: `a/b.md` builds `a/b.html`, and a
/// `readme.md` or `index.md` builds its directory's `index.html`.
pub(crate) fn output_path(source: &str) -> String {
	let (directory, file) = source
		.rsplit_once('/')
		.map_or(("", source), |(directory, file)| (directory, file));
	let stem = markdown_stem(file).unwrap_or(file);
	let file = if stem.eq_ignore_ascii_case("readme") || stem.eq_ignore_ascii_case("index") {
		"index.html".to_owned()
	} else {
		format!("{stem}.html")
	};

	if directory.is_empty() {
		file
	} else {
		format!("{directory}/{file}")
	}
}

/// `path` without its `.md` extension, in any case, if it has one.
fn markdown_stem(path: &str) -> Option<&str> {
	let split = path.len().checked_sub(3)?;
	(path.is_char_boundary(split) && path[split..].eq_ignore_ascii_case(".md"))
		.then(|| &path[..split])
}

/// Points a link written against chapter sources at the built pages.
fn page_url(url: &str) -> String {
	let Target::Relative { path, .. } = classify(url) else {
		return url.to_owned();
	};

	match markdown_stem(path) {
		Some(_) => format!("{}{}", output_path(path), &url[path.len()..]),
		None => url.to_owned(),
	}
}

/// `../` once for every directory `output` is in.
fn root_prefix(output: &str) -> String {
	"../".repeat(output.matches('/').count())
}

fn prepare_output(out: &Path) -> Result<(), Error> {
	match fs::read_dir(out) {
		Ok(mut entries) => {
			if out.join(MARKER).is_file() {
				fs::remove_dir_all(out).map_err(Error::io("clean", out))?;
			} else if entries.next().is_some() {
				return Err(Error::ForeignOutput(out.to_owned()));
			}
		}
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
		Err(error) => return Err(Error::io("read", out)(error)),
	}

	write(
		&out.join(MARKER),
		b"Built by leptoskit_docs. The whole directory is replaced on every build.\n",
	)
}

fn write(path: &Path, contents: &[u8]) -> Result<(), Error> {
	if let Some(parent) = path.parent() {
		fs::create_dir_all(parent).map_err(Error::io("create", parent))?;
	}

	fs::write(path, contents).map_err(Error::io("write", path))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn builds_pages_beside_their_sources() {
		assert_eq!(output_path("readme.md"), "index.html");
		assert_eq!(output_path("guide/README.md"), "guide/index.html");
		assert_eq!(output_path("guide/index.md"), "guide/index.html");
		assert_eq!(output_path("guide/01-install.md"), "guide/01-install.html");
	}

	#[test]
	fn points_chapter_links_at_pages() {
		assert_eq!(page_url("guide/a.md"), "guide/a.html");
		assert_eq!(page_url("../readme.md#top"), "../index.html#top");
		assert_eq!(page_url("a.md?x=1#y"), "a.html?x=1#y");
		assert_eq!(page_url("#top"), "#top");
		assert_eq!(page_url("assets/logo.svg"), "assets/logo.svg");
		assert_eq!(
			page_url("https://example.com/a.md"),
			"https://example.com/a.md"
		);
	}

	#[test]
	fn resolves_relative_paths_inside_the_book() {
		assert_eq!(resolve("guide/a.md", "b.md").as_deref(), Some("guide/b.md"));
		assert_eq!(
			resolve("guide/a.md", "../platforms/./web.md").as_deref(),
			Some("platforms/web.md")
		);
		assert_eq!(resolve("readme.md", "../outside.md"), None);
	}

	#[test]
	fn classifies_link_targets() {
		assert!(matches!(classify("https://example.com"), Target::External));
		assert!(matches!(
			classify("mailto:me@example.com"),
			Target::External
		));
		assert!(matches!(
			classify("//cdn.example.com/x.js"),
			Target::External
		));
		assert!(matches!(classify("/leptoskit/a.html"), Target::Absolute));
		for local in [
			"http://localhost:3000",
			"https://LOCALHOST/",
			"http://app.localhost:8080/path",
			"http://127.0.0.1:3000",
			"http://127.1.2.3",
			"http://0.0.0.0:4400/",
			"http://[::1]:3000/",
			"http://user@localhost",
		] {
			assert!(matches!(classify(local), Target::Local), "{local}");
		}
		for external in [
			"https://localhost.example.com",
			"https://example.com/localhost",
			"https://128.0.0.1",
			"ftp://localhost/file",
		] {
			assert!(matches!(classify(external), Target::External), "{external}");
		}
		assert!(matches!(
			classify("a.md?x#y"),
			Target::Relative {
				path: "a.md",
				anchor: Some("y")
			}
		));
	}

	#[test]
	fn climbs_to_the_root_from_nested_pages() {
		assert_eq!(root_prefix("index.html"), "");
		assert_eq!(root_prefix("guide/a.html"), "../");
		assert_eq!(root_prefix("a/b/c.html"), "../../");
	}
}
