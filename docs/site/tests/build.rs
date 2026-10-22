//! Builds the real leptoskit book, the way the docs workflow publishes it.

use std::fs;

use clap::Parser;
use leptoskit_docs::Summary;
use leptoskit_docs_site::Cli;
use leptoskit_docs_site::book;
use leptoskit_docs_site::site;

fn read(path: impl AsRef<std::path::Path>) -> String {
	fs::read_to_string(path.as_ref())
		.unwrap_or_else(|error| panic!("{}: {error}", path.as_ref().display()))
}

#[test]
fn builds_every_chapter_of_the_book() {
	let out = tempfile::tempdir().unwrap();
	let build = site(book())
		.build(out.path())
		.unwrap_or_else(|error| panic!("{error}"));

	let summary = Summary::parse(&read(book().join("SUMMARY.md"))).unwrap();
	let chapters = summary.pages();
	assert_eq!(build.pages, chapters.len());

	for chapter in chapters {
		let source = chapter.path.as_deref().unwrap();
		let page = match source.strip_suffix(".md").unwrap() {
			"readme" => "index".to_owned(),
			stem if stem.ends_with("/readme") => stem.replace("/readme", "/index"),
			stem => stem.to_owned(),
		};
		let html = read(out.path().join(format!("{page}.html")));
		assert!(html.starts_with("<!DOCTYPE html>"), "{page}");
		assert!(
			html.contains("edit/main/docs/src/"),
			"{page} links to its source"
		);
	}
}

#[test]
fn renders_the_leptoskit_components() {
	let out = tempfile::tempdir().unwrap();
	site(book()).build(out.path()).unwrap();

	let home = read(out.path().join("index.html"));
	assert!(home.contains("<section class=\"lk-hero\">"));
	assert!(home.contains("<span class=\"lk-box lk-box-ota\"></span>"));
	assert!(
		home.contains("href=\"guide/01-installation.html\" class=\"lk-button lk-button-primary\"")
	);
	assert!(
		home.contains("<link rel=\"canonical\" href=\"https://ifiokjr.github.io/leptoskit/\">")
	);
	assert!(home.contains("<link rel=\"stylesheet\" href=\"theme/leptoskit.css\">"));

	let examples = read(out.path().join("examples.html"));
	assert!(examples.contains("<figure class=\"lk-shot lk-shot-wide\">"));
	assert!(examples.contains("width=\"390\" height=\"844\""));

	let brand = read(out.path().join("brand.html"));
	assert!(brand.contains("<figure class=\"lk-logo-tile lk-on-ink\">"));
	assert!(brand.contains("style=\"background: #e5304f;\""));

	assert!(out.path().join("theme/leptoskit.css").is_file());
	assert!(out.path().join("assets/social.png").is_file());
}

#[test]
fn reports_bad_props_on_the_leptoskit_components() {
	let root = tempfile::tempdir().unwrap();
	fs::write(root.path().join("SUMMARY.md"), "[Home](readme.md)\n").unwrap();
	fs::write(
		root.path().join("readme.md"),
		"<Hero>Hi</Hero>\n\n<Shot alt=\"a\" title=\"t\" />\n\n<Logo label=\"l\" />\n\n<Swatch \
		 name=\"n\" />\n\n<Shot src=\"a.png\" alt=\"a\" title=\"t\" wide=\"maybe\" />\n",
	)
	.unwrap();
	fs::write(root.path().join("a.png"), []).unwrap();

	let error = site(root.path())
		.build(root.path().join("dist"))
		.unwrap_err();
	let message = error.to_string();

	for expected in [
		"readme.md:1:1: <Hero> needs a `start` link",
		"readme.md:3:1: <Shot> needs a `src` image",
		"readme.md:5:1: <Logo> needs a `src` image",
		"readme.md:7:1: <Swatch> needs a `color` attribute",
		"readme.md:9:1: `wide=\"maybe\"` on <Shot> is invalid",
	] {
		assert!(message.contains(expected), "{expected}\n\n{message}");
	}
}

#[test]
fn builds_from_the_command_line() {
	let out = tempfile::tempdir().unwrap();
	let target = out.path().join("site");

	Cli::try_parse_from([
		"leptoskit_docs_site",
		"build",
		"--out",
		target.to_str().unwrap(),
	])
	.unwrap()
	.run()
	.unwrap();

	assert!(target.join("index.html").is_file());
	assert!(Cli::try_parse_from(["leptoskit_docs_site", "publish"]).is_err());
}
