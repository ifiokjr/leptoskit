//! Building whole sites from a book on disk.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use leptoskit_docs::Build;
use leptoskit_docs::Error;
use leptoskit_docs::Site;

fn fixture() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/book")
}

fn site(root: impl Into<PathBuf>) -> Site {
	Site::new(root)
		.title("Example")
		.description("An example book.")
		.url("https://example.github.io/project")
		.repository("https://github.com/example/project")
		.edit_url("https://github.com/example/project/edit/main/docs/{path}")
		.logo("assets/dot.svg", "assets/dot.svg")
		.favicon("assets/dot.svg")
		.social_image("assets/dot.svg")
		.stylesheet("assets/extra.css")
}

/// A book of `files` in a fresh directory.
fn book(files: &[(&str, &str)]) -> tempfile::TempDir {
	let root = tempfile::tempdir().unwrap();

	for (path, contents) in files {
		let path = root.path().join(path);
		fs::create_dir_all(path.parent().unwrap()).unwrap();
		fs::write(path, contents).unwrap();
	}

	root
}

fn read(path: impl AsRef<Path>) -> String {
	fs::read_to_string(path.as_ref())
		.unwrap_or_else(|error| panic!("{}: {error}", path.as_ref().display()))
}

fn problems(error: Error) -> Vec<String> {
	match error {
		Error::Docs(diagnostics) => diagnostics.iter().map(ToString::to_string).collect(),
		other => panic!("expected problems in the docs, got {other}"),
	}
}

#[test]
fn rejects_chapter_traversal_without_changing_output() {
	let root = book(&[
		("src/SUMMARY.md", "[Home](readme.md)\n"),
		("src/readme.md", "# Home\n"),
		("outside.md", "# Private\nSensitive content.\n"),
		("outside.html", "keep outside"),
	]);
	let out = root.path().join("dist");
	let site = Site::new(root.path().join("src"));
	site.build(&out).unwrap();
	let original = read(out.join("index.html"));

	for path in [
		"../outside.md".to_owned(),
		root.path().join("outside.md").to_str().unwrap().to_owned(),
	] {
		fs::write(
			root.path().join("src/SUMMARY.md"),
			format!("[Unsafe](<{path}>)\n"),
		)
		.unwrap();
		let errors = problems(site.build(&out).unwrap_err());

		assert!(errors[0].contains("SUMMARY.md:1:1"), "{errors:?}");
		assert_eq!(read(out.join("index.html")), original);
		assert_eq!(read(root.path().join("outside.html")), "keep outside");
		assert!(!read(out.join("_leptoskit/search.json")).contains("Sensitive"));
	}
}

#[cfg(unix)]
#[test]
fn rejects_chapter_symlinks_outside_the_book() {
	use std::os::unix::fs::symlink;

	let root = book(&[
		("src/SUMMARY.md", "[Home](readme.md)\n"),
		("src/readme.md", "# Home\n"),
		("outside/private.md", "# Private\nSensitive content.\n"),
	]);
	symlink("../outside/private.md", root.path().join("src/alias.md")).unwrap();
	symlink("../outside", root.path().join("src/linked")).unwrap();
	let out = root.path().join("dist");
	let site = Site::new(root.path().join("src"));
	site.build(&out).unwrap();
	let original = read(out.join("index.html"));

	for path in ["alias.md", "linked/private.md"] {
		fs::write(
			root.path().join("src/SUMMARY.md"),
			format!("[Unsafe]({path})\n"),
		)
		.unwrap();
		let errors = problems(site.build(&out).unwrap_err());

		assert!(errors[0].contains("outside the book"), "{errors:?}");
		assert_eq!(read(out.join("index.html")), original);
		assert!(!out.join("alias.html").exists());
		assert!(!out.join("linked/private.html").exists());
		assert!(!read(out.join("_leptoskit/search.json")).contains("Sensitive"));
	}
}

#[cfg(unix)]
#[test]
fn rejects_a_summary_symlink_outside_the_book() {
	use std::os::unix::fs::symlink;

	let root = book(&[
		("src/readme.md", "# Home\n"),
		("src-private/SUMMARY.md", "[Private title](readme.md)\n"),
	]);
	symlink(
		"../src-private/SUMMARY.md",
		root.path().join("src/SUMMARY.md"),
	)
	.unwrap();
	let out = root.path().join("dist");

	let error = Site::new(root.path().join("src")).build(&out).unwrap_err();

	assert!(
		matches!(&error, Error::Io { source, .. } if source.kind() == std::io::ErrorKind::PermissionDenied),
		"{error}"
	);
	assert!(!out.exists());
}

#[cfg(unix)]
#[test]
fn accepts_symlinked_roots_and_sources_within_the_book() {
	use std::os::unix::fs::symlink;

	let root = book(&[
		("src/contents.md", "[Guide](alias.md)\n"),
		("src/guide/page.md", "# Guide\n"),
	]);
	symlink("contents.md", root.path().join("src/SUMMARY.md")).unwrap();
	symlink("guide/page.md", root.path().join("src/alias.md")).unwrap();
	symlink("src", root.path().join("book")).unwrap();
	let out = root.path().join("dist");

	let build = Site::new(root.path().join("book")).build(&out).unwrap();

	assert_eq!(build.pages, 1);
	assert!(read(out.join("alias.html")).contains("<h1 id=\"guide\""));
}

#[test]
fn reports_missing_chapters_without_creating_output() {
	let root = book(&[("SUMMARY.md", "[Missing](missing.md)\n")]);
	let out = root.path().join("dist");
	let errors = problems(Site::new(root.path()).build(&out).unwrap_err());

	assert!(
		errors[0].contains("`missing.md` is listed but cannot be read"),
		"{errors:?}"
	);
	assert!(!out.exists());
}

#[cfg(unix)]
#[test]
fn rebuild_replaces_output_symlinks_without_following_them() {
	use std::os::unix::fs::symlink;

	let root = book(&[
		(
			"src/SUMMARY.md",
			"[Home](readme.md)\n- [Guide](guide/page.md)\n",
		),
		("src/readme.md", "# Home\n"),
		("src/guide/page.md", "# Guide\n"),
		("outside/index.html", "keep index"),
		("outside/page.html", "keep page"),
	]);
	let out = root.path().join("dist");
	fs::create_dir(&out).unwrap();
	fs::write(out.join(".leptoskit-docs"), "earlier build").unwrap();
	symlink("../outside/index.html", out.join("index.html")).unwrap();
	symlink("../outside", out.join("guide")).unwrap();

	Site::new(root.path().join("src")).build(&out).unwrap();

	assert_eq!(read(root.path().join("outside/index.html")), "keep index");
	assert_eq!(read(root.path().join("outside/page.html")), "keep page");
	assert!(read(out.join("index.html")).contains("<h1 id=\"home\""));
	assert!(read(out.join("guide/page.html")).contains("<h1 id=\"guide\""));
}

#[test]
fn builds_every_chapter_beside_its_source() {
	let out = tempfile::tempdir().unwrap();
	let build = site(fixture()).build(out.path()).unwrap();

	assert_eq!(build, Build { pages: 4, files: 2 });
	for page in [
		"index.html",
		"guide/install.html",
		"guide/macos.html",
		"reference.html",
		"404.html",
		"assets/dot.svg",
		"assets/extra.css",
		"_leptoskit/site.css",
		"_leptoskit/site.js",
		"_leptoskit/search.json",
	] {
		assert!(out.path().join(page).is_file(), "{page} was not written");
	}
	assert!(
		!out.path().join(".hidden-file").exists(),
		"hidden files are not copied"
	);
	assert!(
		!out.path().join("readme.html").exists(),
		"readme.md builds index.html"
	);
}

#[test]
fn lays_out_a_page() {
	let out = tempfile::tempdir().unwrap();
	site(fixture()).build(out.path()).unwrap();
	let page = read(out.path().join("guide/install.html"));

	insta::assert_snapshot!(page.replace("><", ">\n<"));
}

#[test]
fn links_between_pages_relative_to_each_page() {
	let out = tempfile::tempdir().unwrap();
	site(fixture()).build(out.path()).unwrap();
	let home = read(out.path().join("index.html"));
	let install = read(out.path().join("guide/install.html"));
	let macos = read(out.path().join("guide/macos.html"));

	assert!(
		home.contains("href=\"guide/install.html\" class=\"lk-card\""),
		"{home}"
	);
	assert!(home.contains("href=\"reference.html#options\""), "{home}");
	assert!(
		home.contains("<title>Example</title>"),
		"the home page is titled by the site"
	);
	assert!(home.contains("<link rel=\"canonical\" href=\"https://example.github.io/project/\">"));
	assert!(
		home.contains("content=\"A small book that every part of the site generator touches.\"")
	);
	assert!(home.contains("data-root=\"\""));

	assert!(install.contains("<title>Install · Example</title>"));
	assert!(install.contains("data-root=\"../\""));
	assert!(install.contains("href=\"macos.html#homebrew\""));
	assert!(install.contains("href=\"../index.html\">back home"));
	assert!(install.contains("href=\"../_leptoskit/site.css\""));
	assert!(
		install.contains("<a href=\"../guide/install.html\" aria-current=\"page\">Install</a>")
	);
	assert!(
		install.contains(
			"href=\"https://github.com/example/project/edit/main/docs/guide/install.md\""
		)
	);
	assert!(install.contains(
		"<meta content=\"https://example.github.io/project/assets/dot.svg\" property=\"og:image\">"
	));

	assert!(
		macos.contains("href=\"../guide/install.html\" class=\"lk-pager-previous\""),
		"{macos}"
	);
	assert!(
		macos.contains("href=\"../reference.html\" class=\"lk-pager-next\""),
		"{macos}"
	);
	assert!(macos.contains("<span class=\"lk-nav-draft\">Planned</span>"));
}

#[test]
fn serves_not_found_from_the_absolute_root() {
	let out = tempfile::tempdir().unwrap();
	site(fixture()).build(out.path()).unwrap();
	let not_found = read(out.path().join("404.html"));

	assert!(not_found.contains("data-root=\"/project/\""), "{not_found}");
	assert!(not_found.contains("href=\"/project/index.html\">Back to the start"));
	assert!(!not_found.contains("Edit this page"));

	let local = tempfile::tempdir().unwrap();
	Site::new(fixture()).build(local.path()).unwrap();
	assert!(read(local.path().join("404.html")).contains("data-root=\"/\""));
}

#[test]
fn indexes_every_section_for_search() {
	let out = tempfile::tempdir().unwrap();
	site(fixture()).build(out.path()).unwrap();
	let index: serde_json::Value =
		serde_json::from_str(&read(out.path().join("_leptoskit/search.json"))).unwrap();
	let entries = index.as_array().unwrap();

	let requirements = entries
		.iter()
		.find(|entry| entry["heading"] == "Requirements")
		.expect("an entry per heading");
	assert_eq!(requirements["page"], "Install");
	assert_eq!(requirements["url"], "guide/install.html#requirements");
	assert_eq!(requirements["text"], "You need Rust.");

	let title = entries
		.iter()
		.find(|entry| entry["page"] == "Reference" && entry["heading"].is_null())
		.expect("the title section");
	assert_eq!(title["url"], "reference.html");
}

#[test]
fn replaces_an_earlier_build() {
	let out = tempfile::tempdir().unwrap();
	site(fixture()).build(out.path()).unwrap();
	fs::write(out.path().join("stale.html"), "old").unwrap();

	site(fixture()).build(out.path()).unwrap();

	assert!(!out.path().join("stale.html").exists());
	assert!(out.path().join("index.html").is_file());
}

#[test]
fn refuses_to_clean_a_directory_it_did_not_build() {
	let out = tempfile::tempdir().unwrap();
	fs::write(out.path().join("precious.txt"), "keep me").unwrap();

	let error = site(fixture()).build(out.path()).unwrap_err();

	assert!(matches!(error, Error::ForeignOutput(_)), "{error}");
	assert_eq!(read(out.path().join("precious.txt")), "keep me");
}

#[test]
fn skips_an_output_directory_inside_the_book() {
	let root = book(&[
		("SUMMARY.md", "[Home](readme.md)\n"),
		("readme.md", "# Home\n"),
		("logo.svg", "<svg/>"),
	]);
	let out = root.path().join("dist");

	assert_eq!(Site::new(root.path()).build(&out).unwrap().files, 1);
	assert_eq!(
		Site::new(root.path()).build(&out).unwrap().files,
		1,
		"a rebuild does not copy the last build"
	);
}

#[test]
fn reports_broken_links_and_writes_nothing() {
	let root = book(&[
		("SUMMARY.md", "[Home](readme.md)\n- [Guide](guide.md)\n"),
		(
			"readme.md",
			"# Home\n\n[fine](guide.md#usage) [no page](missing.md) [no heading](guide.md#nope) [no \
			 file](missing.png)\n\n[outside](../elsewhere.md) [absolute](/guide.html) [self](#nowhere) \
			 [external](https://example.com/missing.md)\n\n<Card title=\"t\" href=\"gone.md\" />\n\nServed \
			 at <http://127.0.0.1:3000>, http://localhost:8080, [here](http://[::1]:4400/), and \
			 `http://localhost:3000` in code.\n",
		),
		("guide.md", "# Guide\n\n## Usage\n"),
	]);
	let out = tempfile::tempdir().unwrap();

	let problems = problems(Site::new(root.path()).build(out.path()).unwrap_err());

	insta::assert_snapshot!(problems.join("\n"));
	assert!(
		fs::read_dir(out.path()).unwrap().next().is_none(),
		"nothing is written"
	);
}

#[test]
fn reports_problems_across_every_page() {
	let root = book(&[
		(
			"SUMMARY.md",
			"[Home](readme.md)\n- [Broken](broken.md)\n- [Missing](missing.md)\n- \
			 [Again](readme.md)\n",
		),
		("readme.md", "# Home\n\n<Unknown />\n"),
		("broken.md", "# Broken\n\n<Callout>\n"),
	]);
	let out = tempfile::tempdir().unwrap();

	let problems = problems(Site::new(root.path()).build(out.path()).unwrap_err());

	assert_eq!(problems.len(), 4, "{problems:#?}");
	assert!(problems[0].starts_with("readme.md:3:1: unknown component <Unknown>"));
	assert!(
		problems[1].starts_with("broken.md:4:1: "),
		"{}",
		problems[1]
	);
	assert!(problems[2].starts_with("SUMMARY.md: `missing.md` is listed but cannot be read"));
	assert_eq!(
		problems[3],
		"SUMMARY.md: `readme.md` is listed more than once"
	);
}

#[test]
fn reports_a_malformed_summary() {
	let root = book(&[("SUMMARY.md", "# Summary\n\nJust words.\n")]);

	let problems = problems(
		Site::new(root.path())
			.build(tempfile::tempdir().unwrap().path())
			.unwrap_err(),
	);

	assert_eq!(
		problems,
		["SUMMARY.md:3:1: expected a link to a chapter, such as `[Title](page.md)`"]
	);
}

#[test]
fn needs_a_summary() {
	let root = book(&[("readme.md", "# Home\n")]);

	let error = Site::new(root.path())
		.build(root.path().join("out"))
		.unwrap_err();

	assert!(error.to_string().starts_with("could not read `"), "{error}");
	assert!(error.to_string().contains("SUMMARY.md"), "{error}");
}

#[test]
fn defaults_to_the_title_text_without_a_logo() {
	let root = book(&[
		("SUMMARY.md", "[Home](readme.md)\n"),
		("readme.md", "No heading here.\n"),
	]);
	let out = tempfile::tempdir().unwrap();

	Site::new(root.path())
		.title("Plain")
		.build(out.path())
		.unwrap();
	let home = read(out.path().join("index.html"));

	assert!(home.contains("<span class=\"lk-brand-name\">Plain</span>"));
	assert!(
		home.contains("<title>Plain</title>"),
		"the home page is titled by the site"
	);
	assert!(
		!home.contains("lk-toc"),
		"no table of contents without headings"
	);
	assert!(!home.contains("lk-pager"), "no pager for a single page");
}
