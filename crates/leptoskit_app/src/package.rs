//! Packages the public website, or the complete hosted site, with the actual
//! documentation book.
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use clap::Parser;
use leptoskit_app::landing::Mode;

#[derive(Parser)]
#[command(about = "Build the LeptosKit website, console client, and documentation")]
struct Args {
	/// The deployment's public origin, for documentation canonical links.
	#[arg(long, default_value = "http://localhost:4701")]
	url: String,
	/// An already-built `LeptosKit` CLI executable.
	#[arg(long, default_value = "lk")]
	cli: PathBuf,
	/// Destination for the complete website artifact.
	#[arg(long, default_value = "dist")]
	out: PathBuf,
	/// Build an optimized browser client.
	#[arg(long)]
	release: bool,
	/// Package the public website only: landing page, live example, book, and
	/// schema aliases. No console entrypoint ships, so the artifact can be
	/// published to the public domain as plain static files.
	#[arg(long)]
	public_only: bool,
}

fn main() -> Result<()> {
	let args = Args::parse();
	let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
	let out = if args.out.is_absolute() {
		args.out
	} else {
		root.join(args.out)
	};
	let cli = if args.cli.components().count() > 1 && !args.cli.is_absolute() {
		std::env::current_dir()?.join(args.cli)
	} else {
		args.cli
	};
	let mut build = std::process::Command::new(cli);
	build
		.current_dir(&root)
		.args(["build", "--platform", "web", "--out"])
		.arg(&out);
	if args.release {
		build.arg("--release");
	}
	ensure!(
		build
			.status()
			.context("run the LeptosKit web build")?
			.success(),
		"the web build failed"
	);
	let mode = if args.public_only {
		Mode::Public
	} else {
		Mode::Hosted
	};
	let summary = assemble(&out, &args.url, mode)?;
	println!("{summary}: {}", out.display());
	Ok(())
}

/// Everything after the browser client builds: the pinned Rive runtime, the
/// book, the public schema aliases, and the artifact's HTML entrypoints.
/// Returns the finished artifact's summary line.
fn assemble(out: &Path, url: &str, mode: Mode) -> Result<&'static str> {
	// The web bundler intentionally omits precompressed source files.
	std::fs::create_dir_all(out.join("rive"))?;
	let mut runtime = flate2::read::GzDecoder::new(&include_bytes!("../web/rive/rive.js.gz")[..]);
	std::io::copy(
		&mut runtime,
		&mut std::fs::File::create(out.join("rive/rive.js"))?,
	)?;
	leptoskit_docs_site::site(leptoskit_docs_site::book())
		.url(format!("{}/docs/", url.trim_end_matches('/')))
		.build(out.join("docs"))?;
	if mode == Mode::Public {
		export_schema_aliases(out)?;
		// The public artifact must not keep a previous full packaging's
		// console entrypoint: this output directory may be reused.
		match std::fs::remove_dir_all(out.join("console")) {
			Ok(()) => {}
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
			Err(error) => return Err(error.into()),
		}
		std::fs::write(
			out.join("index.html"),
			leptoskit_app::site::landing_html(url, mode),
		)?;
		Ok("Public website and documentation")
	} else {
		std::fs::create_dir_all(out.join("console"))?;
		std::fs::copy(out.join("index.html"), out.join("console/index.html"))?;
		std::fs::write(
			out.join("index.html"),
			leptoskit_app::site::landing_html(url, mode),
		)?;
		Ok("Website, console and documentation")
	}
}

/// The schema aliases keep their stable public paths (`/schema.json` and
/// `/schemas/`) at the artifact root, beside their copies inside the book at
/// `/docs/`. The old GitHub Pages host redirects to the public domain, so
/// existing editor references keep resolving after this move.
fn export_schema_aliases(out: &Path) -> Result<()> {
	std::fs::copy(out.join("docs/schema.json"), out.join("schema.json"))
		.context("copy the configuration schema to the artifact root")?;
	std::fs::create_dir_all(out.join("schemas"))?;
	for entry in std::fs::read_dir(out.join("docs/schemas"))? {
		let source = entry?.path();
		ensure!(
			source.is_file(),
			"{} is not a schema file",
			source.display()
		);
		std::fs::copy(
			&source,
			out.join("schemas").join(source.file_name().unwrap()),
		)?;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The complete hosted artifact keeps its console entrypoint, and a
	/// reused output directory switches to the public artifact without
	/// leaving the old console entrypoint behind.
	#[test]
	fn repackaging_a_used_directory_replaces_the_console_with_the_public_site() -> Result<()> {
		let out = tempfile::tempdir()?;
		let out = out.path();
		// The web build leaves the client shell; it never clears the
		// destination, so a stale console entrypoint survives until the
		// packager removes it.
		std::fs::write(
			out.join("index.html"),
			"<!doctype html><title>client</title>",
		)?;
		std::fs::create_dir_all(out.join("console"))?;
		std::fs::write(out.join("console/index.html"), "stale console entrypoint")?;

		let hosted = assemble(out, "https://deploy.example.test", Mode::Hosted)?;
		assert_eq!(hosted, "Website, console and documentation");
		let console = std::fs::read_to_string(out.join("console/index.html"))?;
		assert_eq!(console, "<!doctype html><title>client</title>");
		let landing = std::fs::read_to_string(out.join("index.html"))?;
		assert!(landing.contains("href=\"/console/\""));
		assert!(out.join("docs/guide/01-installation.html").is_file());
		assert!(
			!out.join("schema.json").exists(),
			"the hosted artifact serves the book's schemas"
		);

		let public = assemble(out, "https://leptoskit.dev", Mode::Public)?;
		assert_eq!(public, "Public website and documentation");
		assert!(
			!out.join("console").exists(),
			"a reused output directory must not keep the console entrypoint"
		);
		let landing = std::fs::read_to_string(out.join("index.html"))?;
		assert!(
			!landing.contains("/console"),
			"the public page must not offer the console"
		);
		for book_page in [
			"docs/index.html",
			"docs/guide/01-installation.html",
			"docs/guide/29-hosted-deployments.html",
			"docs/_leptoskit/search.json",
			"docs/assets/fonts/skribble/SkribbleNotoSketch-Regular.woff2",
			"docs/theme/skribble.css",
			"rive/rive.js",
		] {
			assert!(out.join(book_page).is_file(), "{book_page} must ship");
		}
		// The schema aliases exist at both public locations with the
		// generated bytes.
		for asset in ["schema.json", "schemas/leptoskit.v0.schema.json"] {
			let exported = std::fs::read(out.join(asset))?;
			let book = std::fs::read(out.join("docs").join(asset))?;
			let committed = std::fs::read(
				Path::new(env!("CARGO_MANIFEST_DIR"))
					.join("../../docs/src")
					.join(asset),
			)?;
			assert_eq!(exported, book, "{asset} must match inside the book");
			assert_eq!(exported, committed, "{asset} must be the generated bytes");
		}
		Ok(())
	}
}
