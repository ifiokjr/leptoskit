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

/// Writes a new project into `path`, creating the directory when needed.
///
/// # Errors
///
/// Returns an error when the app name is not a valid cargo package name, or
/// when any template file already exists: nothing is overwritten.
pub fn run(path: &str, platform: Platform, name: Option<&str>) -> Result<()> {
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
	let files = template(&config, platform)?;

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
fn write_new(path: &Utf8Path, contents: &str) -> Result<()> {
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
fn ensure_package_name(name: &str) -> Result<()> {
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

/// Every file a scaffold writes, relative to the project root, with its
/// contents.
fn template(config: &LeptoskitConfig, platform: Platform) -> Result<Vec<(String, String)>> {
	let name = &config.app.name;
	let mut files = vec![
		(
			leptoskit_config::CONFIG_FILE.to_owned(),
			config.to_toml_string().context("serialize config")?,
		),
		(".gitignore".to_owned(), "/target\n/dist\n".to_owned()),
		("Cargo.toml".to_owned(), server_manifest(name)),
		("src/main.rs".to_owned(), MAIN_RS.to_owned()),
		("client/Cargo.toml".to_owned(), client_manifest(name)),
		("client/src/lib.rs".to_owned(), CLIENT_LIB_RS.to_owned()),
		("public/index.html".to_owned(), index_html(name)),
	];
	// Native projects carry their Apple host as a Swift Package: no Podfile,
	// no generated Xcode project — `open hosts/apple` and Xcode understands
	// the folder as-is. See docs/src/platforms/host-projects.md for the
	// design.
	if platform == Platform::Native {
		let [package_swift, main_swift] = native_paths(name);
		files.push((
			package_swift,
			apple_package_swift(name, &config.native.package_id),
		));
		files.push((main_swift, APPLE_MAIN_SWIFT.to_owned()));
	}
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
/// pinned to the wasm-bindgen this CLI embeds.
fn client_manifest(name: &str) -> String {
	let package = client_package(name);
	let wasm_bindgen = wasm_bindgen_shared::SCHEMA_VERSION;
	format!(
		r#"[package]
name = "{package}"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
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
  </head>
  <body>
    <main id="app">loading…</main>
    <script type="module">
      import init, {{ greeting }} from "./pkg/{lib}.js";

      await init();
      document.getElementById("app").textContent = greeting("{name}");
    </script>
  </body>
</html>
"#
	)
}

/// The Apple host package manifest. The executable is real and builds with
/// `swift build` today; the runtime bridge lines are where the wasm host's
/// C ABI lands (tracked with the native backend work).
fn apple_package_swift(name: &str, package_id: &str) -> String {
	format!(
		r#"// swift-tools-version:6.0
// The leptoskit Apple host: a Swift Package, not an Xcode project or a
// CocoaPods integration. `swift build` compiles this skeleton as-is, and
// Xcode opens the folder directly (`open hosts/apple`).
//
// The app identity lives in leptoskit.toml: package id `{package_id}`.
import PackageDescription

let package = Package(
    name: "{name}",
    platforms: [.macOS(.v14), .iOS(.v17)],
    targets: [
        .executableTarget(
            name: "{name}",
            path: "Sources/{name}"
            // The leptoskit runtime links here as a static library target
            // with a C header, once the native backend ABI lands:
            //
            //   .linkedLibrary("leptoskit_runtime")
            //
            // The host then boots the wasm bundle, pumps events, and checks
            // for code push updates — see
            // https://ifiokjr.github.io/leptoskit/platforms/host-projects.html
        )
    ]
)
"#
	)
}

/// The host entry point skeleton.
const APPLE_MAIN_SWIFT: &str = r#"// The leptoskit host entry point.
//
// Three responsibilities, and only three: boot the wasm runtime over the
// app bundle, forward platform events, and check for code push updates on
// launch. The interface itself is a wasm module downloaded over the air —
// this file changes only when the host's native capabilities change.
//
// Runtime bridge lands with the native backend ABI; this skeleton builds
// with `swift build` today so the package shape is continuously verified.
print("leptoskit host skeleton: the runtime bridge attaches here")
"#;

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

use wasm_bindgen::prelude::wasm_bindgen;

/// A greeting computed in wasm, shown by `public/index.html`.
#[wasm_bindgen]
pub fn greeting(name: &str) -> String {
	format!("hello from {name}, running in wasm")
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

/// The files only a native scaffold writes: the Swift Package host.
pub fn native_template_files(name: &str) -> Vec<String> {
	native_paths(name).to_vec()
}

fn native_paths(name: &str) -> [String; 2] {
	[
		"hosts/apple/Package.swift".to_owned(),
		format!("hosts/apple/Sources/{name}/main.swift"),
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
