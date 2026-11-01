//! `leptoskit build --platform extension`: the app as a browser extension.
//!
//! The web client is built once with the app's `extension` feature, then
//! packaged for each browser family in `[extension].browsers`:
//!
//! ```text
//! .leptoskit/extension/
//! ├── chromium/            the unpacked extension: manifest.json, pages, pkg/, assets/
//! ├── chromium.zip         what the Chrome Web Store and Edge Add-ons take
//! ├── firefox/  firefox.zip
//! ├── firefox-source.zip   the source addons.mozilla.org asks for
//! └── safari/              copied into the Apple apps' Safari extension
//! ```

pub mod dev;
pub mod manifest;
pub mod pages;
pub mod test;
pub mod zip;

use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::Browser;
use leptoskit_config::ExtensionConfig;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;

use self::manifest::DevManifest;
use self::manifest::ManifestInput;
use self::pages::Module;

/// Where extension builds go, relative to the project root.
pub const OUTPUT: &str = ".leptoskit/extension";

/// The icon sizes stores and browsers use, read from `assets/icon-<size>.png`.
pub const ICON_SIZES: [u32; 4] = [16, 32, 48, 128];

/// One browser family's package.
#[derive(Debug, Clone)]
pub struct Package {
	pub browser: Browser,
	/// The unpacked extension.
	pub dir: PathBuf,
	/// The zip for the stores; Safari's package goes into the app instead.
	pub zip: Option<PathBuf>,
}

/// What an extension build produced.
#[derive(Debug, Clone)]
pub struct Built {
	pub packages: Vec<Package>,
	/// The source archive for addons.mozilla.org.
	pub source: Option<PathBuf>,
}

/// `leptoskit build --platform extension`.
///
/// # Errors
///
/// When the configuration or the build fails.
pub fn command(release: bool) -> Result<()> {
	let built = build(release, None)?;

	for package in &built.packages {
		match &package.zip {
			Some(zip) => {
				println!(
					"built the {} extension in {} ({})",
					package.browser,
					package.dir.display(),
					zip.display()
				);
			}
			None => {
				println!(
					"built the {} extension in {}, for the Apple apps to embed",
					package.browser,
					package.dir.display()
				);
			}
		}
	}

	if let Some(source) = &built.source {
		println!(
			"wrote the source archive addons.mozilla.org asks for: {}",
			source.display()
		);
	}

	Ok(())
}

/// Builds and packages the extension. `dev` builds an unpacked Chromium
/// package for `leptoskit dev` instead.
///
/// # Errors
///
/// When `[extension]` is missing, a configured route doesn't exist, or the
/// build fails.
pub fn build(release: bool, dev: Option<&DevManifest>) -> Result<Built> {
	build_for(release, dev, &[])
}

/// Builds and packages the extension for `browsers` only, or for every
/// configured browser when `browsers` is empty, as the native builds do to
/// embed Safari's package.
///
/// # Errors
///
/// When `[extension]` is missing, a configured route doesn't exist, or the
/// build fails.
pub fn build_for(release: bool, dev: Option<&DevManifest>, browsers: &[Browser]) -> Result<Built> {
	let (config, root) = LeptoskitConfig::load()?;
	config.ensure_platform_enabled(Platform::Extension)?;
	let extension = config
		.extension
		.clone()
		.context("add an [extension] table to leptoskit.toml to build an extension")?;
	check_routes(&root, &config, &extension)?;
	anyhow::ensure!(
		!config.codepush.enabled,
		"extension stores forbid code downloaded after review, so code push can't be on for an \
		 extension build"
	);

	let output = crate::build::run(
		release,
		Some(Platform::Extension),
		None,
		&format!("{OUTPUT}/build"),
	)?;
	let client = output
		.client
		.context("an extension needs the app's wasm client crate")?;
	let pkg = output.out_dir.join("pkg").into_std_path_buf();
	anyhow::ensure!(
		pkg.join(format!("{}_bg.wasm", client.lib_name)).is_file(),
		"the build left no {}_bg.wasm in {}",
		client.lib_name,
		pkg.display()
	);
	let module = Module {
		stem: &client.lib_name,
	};
	let assets = root.join("assets");
	let output_dir = root.join(OUTPUT);
	// `app.icon` makes every size; without it, the app's own
	// `assets/icon-<size>.png` files are used.
	let generated_icons = output_dir.join("icons");

	let generated = crate::icons::write(
		&root,
		&config,
		crate::icons::Platform::Extension,
		&generated_icons,
	)?;
	let icons: Vec<u32> = ICON_SIZES
		.into_iter()
		.filter(|size| generated || assets.join(format!("icon-{size}.png")).is_file())
		.collect();
	let generated_icons = generated.then_some(generated_icons.as_path());

	let plugin_permissions = plugin_permissions(&root, &client.name)?;
	let browsers: Vec<Browser> = if dev.is_some() {
		vec![Browser::Chromium]
	} else if browsers.is_empty() {
		extension.browsers.clone()
	} else {
		browsers.to_vec()
	};

	let mut packages = Vec::new();

	for &browser in &browsers {
		let dir = output_dir.join(if dev.is_some() {
			"dev/chromium".to_owned()
		} else {
			browser.name().to_owned()
		});

		let input = ManifestInput {
			browser,
			config: &config,
			extension: &extension,
			plugin_permissions: &plugin_permissions,
			icons: &icons,
			dev,
		};
		write_package(&dir, &input, &module, &pkg, &assets, generated_icons)?;
		let zip = if dev.is_none() && browser != Browser::Safari {
			let zip = output_dir.join(format!("{}.zip", browser.name()));
			std::fs::write(&zip, zip::archive(&zip::files_under(&dir)?)?)
				.with_context(|| format!("could not write {}", zip.display()))?;
			Some(zip)
		} else {
			None
		};

		packages.push(Package { browser, dir, zip });
	}

	let source = if dev.is_none() && browsers.contains(&Browser::Firefox) {
		let path = output_dir.join("firefox-source.zip");
		std::fs::write(&path, source_archive(&root, &config, release)?)
			.with_context(|| format!("could not write {}", path.display()))?;
		Some(path)
	} else {
		None
	};

	Ok(Built { packages, source })
}

/// Writes one browser family's unpacked extension into `dir`, replacing
/// whatever was there.
fn write_package(
	dir: &Path,
	input: &ManifestInput<'_>,
	module: &Module<'_>,
	pkg: &Path,
	assets: &Path,
	icons: Option<&Path>,
) -> Result<()> {
	if dir.exists() {
		std::fs::remove_dir_all(dir)
			.with_context(|| format!("could not clear {}", dir.display()))?;
	}

	std::fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
	let write = |name: &str, contents: &str| -> Result<()> {
		std::fs::write(dir.join(name), contents)
			.with_context(|| format!("could not write {}", dir.join(name).display()))
	};
	let extension = input.extension;
	let browser = input.browser;
	let title = manifest::display_name(input);
	let stylesheet = assets.join("app.css").is_file();

	let mut manifest = serde_json::to_string_pretty(&manifest::manifest(input))?;
	manifest.push('\n');
	write("manifest.json", &manifest)?;

	for page in extension.pages() {
		if !extension.has_page(&page, browser) {
			continue;
		}

		write(
			&format!("{}.html", page.name),
			&pages::page_html(page.name, &title, stylesheet),
		)?;
		write(
			&format!("{}.js", page.name),
			&pages::page_js(module, page.name, page.route),
		)?;
	}

	if extension.background.is_some() || input.dev.is_some() {
		let events = extension
			.background
			.as_ref()
			.map(|background| background.events.as_slice());
		write(
			"background.js",
			&pages::background_js(module, events, input.dev.map(|dev| dev.server.as_str())),
		)?;
	}

	let mut overlay = false;

	for script in &extension.content_scripts {
		if !extension.includes(script.browsers.as_deref(), browser) {
			continue;
		}

		write(
			&format!("content-{}.js", script.name),
			&pages::content_js(&script.name, &script.route),
		)?;
		overlay = true;
	}

	if overlay {
		write(
			"overlay.html",
			&pages::page_html("overlay", &title, stylesheet),
		)?;
		write("overlay.js", &pages::overlay_js(module))?;
	}

	copy_dir(pkg, &dir.join("pkg"))?;

	if assets.is_dir() {
		copy_dir(assets, &dir.join("assets"))?;
	}

	if let Some(icons) = icons {
		copy_dir(icons, &dir.join("assets"))?;
	}

	Ok(())
}

/// The extension permissions the client's plugins declare in
/// `[package.metadata.leptoskit.runtime] extension-permissions`, from every
/// package in its dependency graph, sorted.
fn plugin_permissions(root: &Path, client: &str) -> Result<Vec<String>> {
	let output = std::process::Command::new("cargo")
		.args(["metadata", "--format-version", "1"])
		.current_dir(root)
		.output()
		.context("could not run cargo metadata")?;
	anyhow::ensure!(
		output.status.success(),
		"cargo metadata failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);
	let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
	Ok(declared_permissions(&metadata, client))
}

/// Walks the resolved graph from `client` and collects what its packages
/// declare.
fn declared_permissions(metadata: &serde_json::Value, client: &str) -> Vec<String> {
	use std::collections::BTreeSet;
	use std::collections::HashMap;

	let packages: HashMap<&str, &serde_json::Value> = metadata["packages"]
		.as_array()
		.into_iter()
		.flatten()
		.filter_map(|package| Some((package["id"].as_str()?, package)))
		.collect();
	let nodes: HashMap<&str, Vec<&str>> = metadata["resolve"]["nodes"]
		.as_array()
		.into_iter()
		.flatten()
		.filter_map(|node| {
			let deps = node["deps"]
				.as_array()
				.into_iter()
				.flatten()
				.filter_map(|dep| dep["pkg"].as_str())
				.collect();
			Some((node["id"].as_str()?, deps))
		})
		.collect();
	let mut stack: Vec<&str> = packages
		.iter()
		.filter(|(_, package)| package["name"] == client)
		.map(|(id, _)| *id)
		.collect();
	let mut seen = BTreeSet::new();
	let mut permissions = BTreeSet::new();

	while let Some(id) = stack.pop() {
		if !seen.insert(id) {
			continue;
		}

		if let Some(package) = packages.get(id) {
			for permission in package["metadata"]["leptoskit"]["runtime"]["extension-permissions"]
				.as_array()
				.into_iter()
				.flatten()
				.filter_map(serde_json::Value::as_str)
			{
				permissions.insert(permission.to_owned());
			}
		}

		stack.extend(nodes.get(id).into_iter().flatten().copied());
	}

	permissions.into_iter().collect()
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
	for entry in walkdir::WalkDir::new(from) {
		let entry = entry.with_context(|| format!("could not read {}", from.display()))?;
		let target = to.join(entry.path().strip_prefix(from).expect("under its root"));

		if entry.file_type().is_dir() {
			std::fs::create_dir_all(&target)
				.with_context(|| format!("could not create {}", target.display()))?;
		} else if entry.file_type().is_file()
			// Stores compress packages themselves.
			&& !matches!(
				entry.path().extension().and_then(|extension| extension.to_str()),
				Some("br" | "gz")
			) {
			std::fs::copy(entry.path(), &target)
				.with_context(|| format!("could not copy {}", entry.path().display()))?;
		}
	}

	Ok(())
}

/// Every route `[extension]` opens has to be one of the app's routes, so a
/// typo fails the build rather than the popup.
fn check_routes(root: &Path, config: &LeptoskitConfig, extension: &ExtensionConfig) -> Result<()> {
	let Some(spec) = crate::build::router_spec(config) else {
		return Ok(());
	};
	let dir = spec
		.options
		.get("dir")
		.and_then(|dir| dir.as_str())
		.map_or_else(
			|| leptoskit_config::RouterConfig::default().dir,
			PathBuf::from,
		);
	let generated = leptoskit_router_codegen::generate(&root.join(&dir))
		.map_err(|diagnostics| anyhow::anyhow!("{diagnostics}"))?;
	let patterns: Vec<&str> = generated
		.routes
		.iter()
		.filter(|route| route.kind == leptoskit_router_codegen::RouteKind::Page)
		.map(|route| route.pattern.as_str())
		.collect();
	let configured = extension
		.pages()
		.map(|page| {
			// `extension.popup`, or `extension.devtools_page.panel` for the
			// setting that isn't the table's `route`.
			let surface = page.setting.strip_suffix(".route").unwrap_or(page.setting);
			(surface.to_owned(), page.route.to_owned())
		})
		.chain(extension.content_scripts.iter().map(|script| {
			(
				format!("content script `{}`", script.name),
				script.route.clone(),
			)
		}));

	for (surface, route) in configured {
		let path = route.split(['?', '#']).next().unwrap_or_default();
		anyhow::ensure!(
			patterns.iter().any(|pattern| route_matches(pattern, path)),
			"{surface} opens `{route}`, which isn't one of the app's routes in {}",
			dir.display()
		);
	}

	Ok(())
}

/// Whether `path` matches a route pattern such as `/projects/:project` or
/// `/help/*topic`.
fn route_matches(pattern: &str, path: &str) -> bool {
	let pattern: Vec<&str> = pattern.split('/').filter(|part| !part.is_empty()).collect();
	let path: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
	let mut index = 0;

	for (position, part) in pattern.iter().enumerate() {
		if let Some(rest) = part.strip_prefix('*') {
			let optional = rest.ends_with('?');

			return optional || path.len() > index || position + 1 < pattern.len();
		}

		if part.starts_with(':') && part.ends_with('?') {
			// An optional segment: skip it when what follows still matches.
			let rest = pattern[position + 1..].join("/");
			let remaining = path[index..].join("/");

			if route_matches(&format!("/{rest}"), &format!("/{remaining}")) {
				return true;
			}

			if index >= path.len() {
				return false;
			}

			index += 1;
			continue;
		}

		let Some(segment) = path.get(index) else {
			return false;
		};

		if !part.starts_with(':') && part != segment {
			return false;
		}

		index += 1;
	}

	index == path.len()
}

/// The project's source, for reviewers to rebuild the Firefox package from:
/// every file git tracks (or, outside git, everything but build output and
/// secrets), plus a `BUILD.md` with the exact commands and tool versions.
fn source_archive(root: &Path, config: &LeptoskitConfig, release: bool) -> Result<Vec<u8>> {
	let mut files: Vec<(String, Vec<u8>)> = Vec::new();

	for name in source_files(root) {
		let contents = std::fs::read(root.join(&name))
			.with_context(|| format!("could not read {name} for the source archive"))?;
		files.push((name, contents));
	}

	let rustc = tool_version("rustc");
	let cargo = tool_version("cargo");
	let build = format!(
		"# Building {name} for Firefox\n\nThis archive is the source of the extension's \
		 `firefox.zip`. Its WebAssembly is compiled from the Rust in this archive.\n\n## \
		 Tools\n\n- rustc: {rustc}\n- cargo: {cargo}\n- leptoskit: {leptoskit}, from \
		 <https://github.com/ifiokjr/leptoskit>\n- The `wasm32-unknown-unknown` Rust target: \
		 `rustup target add wasm32-unknown-unknown`\n\n## Build\n\n```sh\ncargo install \
		 leptoskit_cli --version {leptoskit} --locked\nleptoskit build --platform \
		 extension{release}\n```\n\nThe package is written to \
		 `.leptoskit/extension/firefox/`, and zipped as `.leptoskit/extension/firefox.zip`. \
		 Zips are deterministic: the same source builds the same bytes.\n",
		name = config.app.name,
		leptoskit = env!("CARGO_PKG_VERSION"),
		release = if release { " --release" } else { "" },
	);
	files.retain(|(name, _)| name != "BUILD.md");

	files.push(("BUILD.md".to_owned(), build.into_bytes()));
	files.sort_by(|left, right| left.0.cmp(&right.0));
	zip::archive(&files)
}

fn tool_version(tool: &str) -> String {
	std::process::Command::new(tool)
		.arg("--version")
		.output()
		.ok()
		.filter(|output| output.status.success())
		.map_or_else(
			|| "unknown".to_owned(),
			|output| String::from_utf8_lossy(&output.stdout).trim().to_owned(),
		)
}

/// The project's source files, relative to the root.
fn source_files(root: &Path) -> Vec<String> {
	let listed = crate::git::command(root)
		.args([
			"ls-files",
			"--cached",
			"--others",
			"--exclude-standard",
			"-z",
			".",
		])
		.output()
		.ok()
		.filter(|output| output.status.success());

	let mut files: Vec<String> = match listed {
		Some(output) => {
			output
				.stdout
				.split(|byte| *byte == 0)
				.filter(|name| !name.is_empty())
				.map(|name| String::from_utf8_lossy(name).into_owned())
				.filter(|name| root.join(name).is_file())
				.collect()
		}
		None => {
			walkdir::WalkDir::new(root)
				.into_iter()
				.filter_entry(|entry| {
					let name = entry.file_name().to_string_lossy();
					!matches!(
						name.as_ref(),
						"target" | ".leptoskit" | "dist" | "node_modules" | ".git"
					)
				})
				.filter_map(Result::ok)
				.filter(|entry| entry.file_type().is_file())
				.filter_map(|entry| {
					entry
						.path()
						.strip_prefix(root)
						.ok()
						.map(|path| path.to_string_lossy().replace('\\', "/"))
				})
				.collect()
		}
	};

	// Secrets never go to a store, whatever git tracks.
	files.retain(|name| {
		let file = name.rsplit('/').next().unwrap_or(name).to_ascii_lowercase();
		let extension = Path::new(&file)
			.extension()
			.and_then(|extension| extension.to_str());
		!file.starts_with(".env") && !matches!(extension, Some("pem" | "p8" | "p12" | "key"))
	});
	files.sort();
	files
}

#[cfg(test)]
mod tests;
