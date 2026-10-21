//! `leptoskit new`: scaffolds a project from the embedded template.
//!
//! The template is deliberately small but complete: `leptoskit build` works on
//! it straight away. It is a cargo workspace whose root package is the server
//! binary (named after the app, which is what `leptoskit build` compiles) and
//! whose `client` member is the `cdylib` wasm client, plus a `public/` page
//! that loads the bound client.

use std::io::Write;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use camino::Utf8Path;
use camino::Utf8PathBuf;
use leptoskit_config::AppConfig;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::NativeConfig;
use leptoskit_config::Platform;

/// Writes a new project into `path`, creating the directory when needed. The
/// project depends on the published leptoskit release.
///
/// # Errors
///
/// Returns an error when the app name is not a valid cargo package name, or
/// when any template file already exists: nothing is overwritten.
pub fn run(path: &str, platform: Platform, name: Option<&str>) -> Result<()> {
	run_with_sdk(path, platform, name, None)
}

/// Writes a new project like [`run`], depending on the leptoskit crates of the
/// local SDK repository at `sdk` instead of the published release, so the
/// scaffold can use features that have not been released yet.
///
/// # Errors
///
/// Returns an error as [`run`] does, and when `sdk` is not a leptoskit
/// repository.
pub fn run_with_sdk(
	path: &str,
	platform: Platform,
	name: Option<&str>,
	sdk: Option<&str>,
) -> Result<()> {
	if platform == Platform::Native {
		return crate::native::scaffold::run(path, name, sdk);
	}
	let leptoskit = sdk.map(sdk_leptoskit_crate).transpose()?;
	std::fs::create_dir_all(path).with_context(|| format!("could not create {path}"))?;
	// Canonicalized so `leptoskit new .` names the app after the directory.
	let dir = std::fs::canonicalize(path).with_context(|| format!("could not resolve {path}"))?;
	let dir = Utf8PathBuf::from_path_buf(dir)
		.map_err(|dir| anyhow::anyhow!("{} is not UTF-8", dir.display()))?;
	let app_name = match name {
		Some(name) => name,
		None => {
			dir.file_name()
				.context("the project directory has no name")?
		}
	};
	ensure_package_name(app_name)?;

	let config = LeptoskitConfig {
		app: AppConfig {
			name: app_name.to_owned(),
			default_platform: platform,
			..AppConfig::default()
		},
		native: NativeConfig {
			package_id: leptoskit_config::default_package_id(app_name),
			..NativeConfig::default()
		},
		..LeptoskitConfig::default()
	};
	let files = template(&config, leptoskit.as_deref())?;

	// Check everything first so a conflict leaves no half-written scaffold;
	// `create_new` below still refuses to clobber a file created meanwhile.
	for (relative, _) in &files {
		let path = dir.join(relative);
		anyhow::ensure!(!path.exists(), "{path} already exists");
	}
	for (relative, contents) in &files {
		write_new(&dir.join(relative), contents)?;
	}

	println!("created {dir} ({platform} project)");
	// Echoes the alias the user typed, so `lk new` points at `lk build`.
	println!("next: cd {dir} && {} build", crate::cli::invoked_name());
	Ok(())
}

/// Creates `path` (and its parent directories) with `contents`, failing when
/// the file already exists.
pub(crate) fn write_new(path: &Utf8Path, contents: &str) -> Result<()> {
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent).with_context(|| format!("could not create {parent}"))?;
	}
	let mut file = std::fs::OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(path)
		.with_context(|| format!("could not create {path}"))?;
	file.write_all(contents.as_bytes())
		.with_context(|| format!("could not write {path}"))
}

/// Rejects names cargo would refuse as a package name.
pub(crate) fn ensure_package_name(name: &str) -> Result<()> {
	let mut characters = name.chars();
	let valid = characters
		.next()
		.is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
		&& characters.all(|character| {
			character.is_ascii_alphanumeric() || character == '_' || character == '-'
		});
	anyhow::ensure!(
		valid,
		"`{name}` is not a valid cargo package name: use ASCII letters, digits, `_`, and `-`, \
		 starting with a letter or `_` (or pass --name)"
	);
	Ok(())
}

/// The client crate's package name for an app.
fn client_package(app_name: &str) -> String {
	format!("{app_name}_client")
}

/// The client's library name, which names its wasm-bindgen outputs.
fn client_lib(app_name: &str) -> String {
	client_package(app_name).replace('-', "_")
}

/// The `crates/leptoskit` directory of the SDK repository at `sdk`.
fn sdk_leptoskit_crate(sdk: &str) -> Result<Utf8PathBuf> {
	let root = std::fs::canonicalize(sdk).context("resolve --sdk-path")?;
	let root = Utf8PathBuf::from_path_buf(root)
		.map_err(|root| anyhow::anyhow!("{} is not UTF-8", root.display()))?;
	let leptoskit = root.join("crates").join("leptoskit");
	anyhow::ensure!(
		leptoskit.join("Cargo.toml").is_file(),
		"--sdk-path {root} has no crates/leptoskit"
	);
	Ok(leptoskit)
}

/// Every file a scaffold writes, relative to the project root, with its
/// contents. `leptoskit` is the SDK's leptoskit crate when the scaffold
/// depends on a local SDK.
fn template(
	config: &LeptoskitConfig,
	leptoskit: Option<&Utf8Path>,
) -> Result<Vec<(String, String)>> {
	let name = &config.app.name;
	let files = vec![
		(
			leptoskit_config::CONFIG_FILE.to_owned(),
			config.to_toml_string().context("serialize config")?,
		),
		(".gitignore".to_owned(), "/target\n/dist\n".to_owned()),
		("Cargo.toml".to_owned(), server_manifest(name)),
		("src/main.rs".to_owned(), MAIN_RS.to_owned()),
		(
			"client/Cargo.toml".to_owned(),
			client_manifest(name, leptoskit),
		),
		("client/src/lib.rs".to_owned(), CLIENT_LIB_RS.to_owned()),
		("public/index.html".to_owned(), index_html(name)),
	];
	Ok(files)
}

/// The root manifest: the server package, and the workspace holding the
/// client.
fn server_manifest(name: &str) -> String {
	format!(
		r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
publish = false

[workspace]
members = ["client"]

[dependencies]
"#
	)
}

/// The client manifest: a `cdylib`, which is how `leptoskit build` finds it,
/// pinned to the wasm-bindgen this CLI embeds. With a local SDK, leptoskit
/// resolves from its crate directory and keeps the version requirement, so
/// the manifest still names the release it matches.
fn client_manifest(name: &str, leptoskit: Option<&Utf8Path>) -> String {
	let package = client_package(name);
	let wasm_bindgen = wasm_bindgen_shared::SCHEMA_VERSION;
	let version = env!("CARGO_PKG_VERSION");
	// Written as a TOML string so a Windows path keeps its backslashes.
	let path = leptoskit
		.map(|crate_dir| format!(", path = {}", toml::Value::from(crate_dir.as_str())))
		.unwrap_or_default();
	format!(
		r#"[package]
name = "{package}"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
leptos = {{ version = "0.8", features = ["csr"] }}
leptoskit = {{ version = "={version}"{path}, features = ["web"] }}
# Pinned exactly: leptoskit binds the client with an embedded wasm-bindgen
# {wasm_bindgen}, and wasm-bindgen only accepts wasm built against its own version.
wasm-bindgen = "={wasm_bindgen}"
"#
	)
}

/// The page that loads the bound client from `pkg/`.
fn index_html(name: &str) -> String {
	let lib = client_lib(name);
	format!(
		r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>{name}</title>
    <style>body {{ margin: 0; }}</style>
  </head>
  <body>
    <script type="module">
      import init from "./pkg/{lib}.js";

      await init({{ module_or_path: "./pkg/{lib}_bg.wasm" }});
    </script>
  </body>
</html>
"#
	)
}

/// The starter server binary. It has no dependencies so a fresh project
/// builds in seconds; `leptoskit serve` serves `dist/` in the meantime.
const MAIN_RS: &str = r#"//! The server binary `leptoskit build` compiles alongside the wasm client.
//!
//! Replace this with your axum server (for example with `leptos_axum` for
//! server rendering). Until then, `leptoskit serve` serves the built `dist/`.

fn main() {
	println!("run `leptoskit serve` to serve dist/");
}
"#;

/// The starter wasm client.
const CLIENT_LIB_RS: &str = r#"//! The wasm client: `leptoskit build` compiles it for
//! `wasm32-unknown-unknown` and writes the wasm-bindgen glue to `dist/pkg/`.

use leptos::prelude::*;
use leptoskit::prelude::{Button, Element, Text, View};
use wasm_bindgen::prelude::wasm_bindgen;

/// The same component tree can be committed to a native surface.
pub fn screen(count: i32, increment: impl Fn() + Send + Sync + 'static) -> Element {
	View()
		.class("min-h-screen items-center justify-center gap-6 p-6 bg-slate-950 text-white")
		.child(Text("Hello, leptoskit").class("text-3xl font-bold"))
		.child(Text(format!("Count: {count}")).class("text-xl"))
		.child(Button("Add one").class("rounded-lg bg-indigo-500 px-6 py-3 font-semibold").on_press(increment))
}

/// Mounts the shared screen through the Leptos web adapter.
#[wasm_bindgen(start)]
pub fn start() {
	leptos::mount::mount_to_body(|| {
		let count = RwSignal::new(0);
		move || leptoskit::web::render(screen(count.get(), move || count.update(|value| *value += 1)))
	});
}
"#;

/// The files a scaffold always writes, relative to the project root.
pub fn template_files() -> &'static [&'static str] {
	&[
		leptoskit_config::CONFIG_FILE,
		".gitignore",
		"Cargo.toml",
		"src/main.rs",
		"client/Cargo.toml",
		"client/src/lib.rs",
		"public/index.html",
	]
}

/// Ensures the scaffold wrote every file it promises.
pub fn verify_scaffold(dir: &Path) -> Result<()> {
	for file in template_files() {
		let path = dir.join(file);
		anyhow::ensure!(path.is_file(), "scaffold is missing {file}");
	}
	Ok(())
}
