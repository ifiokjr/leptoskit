//! Packages the browser client and the actual documentation book together.
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use clap::Parser;

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
	// The web bundler intentionally omits precompressed source files.
	let mut runtime = flate2::read::GzDecoder::new(&include_bytes!("../web/rive/rive.js.gz")[..]);
	std::io::copy(
		&mut runtime,
		&mut std::fs::File::create(out.join("rive/rive.js"))?,
	)?;
	leptoskit_docs_site::site(leptoskit_docs_site::book())
		.url(format!("{}/docs/", args.url.trim_end_matches('/')))
		.build(out.join("docs"))?;
	std::fs::create_dir_all(out.join("console"))?;
	std::fs::copy(out.join("index.html"), out.join("console/index.html"))?;
	std::fs::write(
		out.join("index.html"),
		leptoskit_app::site::landing_html(&args.url),
	)?;
	println!("Website, console and documentation: {}", out.display());
	Ok(())
}
