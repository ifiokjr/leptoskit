//! `leptoskit build`: one pipeline for every platform.
//!
//! The pipeline mirrors what cargo-leptos does — compile the server, compile
//! the wasm client, run wasm-bindgen, optionally optimize, lay out assets:
//!
//! - cargo always runs for the server and the client, one after the other: both
//!   share the target directory and cargo's lock on it, and cargo's own
//!   incremental cache makes an unchanged build a sub-second no-op;
//! - wasm-bindgen (and, in release, `wasm-opt`) is content-fingerprinted
//!   through [`crate::cache`] on the compiled wasm, so an unchanged compile
//!   skips it — unless its outputs went missing or the output directory moved;
//! - the `public/` tree is copied file by file, skipping files whose content
//!   already matches the copy in the output;
//! - with `bundle.split`, wasm-split cuts the client into lazily loaded chunks
//!   before wasm-bindgen runs (see [`crate::split`]);
//! - release web builds precompress the output with brotli and gzip, and every
//!   release build reports the client bundle's size against `bundle.budget_kib`
//!   (see [`crate::compress`] and [`crate::report`]).
//!
//! `app.render` decides the features each side compiles with (see
//! [`crate::render`]), and in a server-rendering mode the server binary the
//! build produced is recorded for `leptoskit serve`.

use std::collections::BTreeMap;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;

use anyhow::Context;
use anyhow::Result;
use camino::Utf8Path;
use camino::Utf8PathBuf;
use leptoskit_config::BundleConfig;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Optimize;
use leptoskit_config::Platform;
use leptoskit_plugin::LeptoskitPlugin as _;

use crate::cache::BuildCache;
use crate::cache::hash_bytes;
use crate::compress;
use crate::compress::PrecompressCache;
use crate::render;
use crate::report::BundleReport;
use crate::server;
use crate::server::BuiltServer;
use crate::split;
use crate::ui::BOLD;
use crate::ui::DIM;
use crate::wasm::CargoOptions;
use crate::wasm::ClientInfo;
use crate::wasm::{self};

/// What a build produced, for the commands that run it.
#[derive(Debug, Clone)]
pub struct BuildOutput {
	/// The built site directory.
	pub out_dir: Utf8PathBuf,
	/// The wasm client, when the platform has one.
	pub client: Option<ClientInfo>,
	/// The server executable, when cargo reported exactly one for the server
	/// package (or one named after it).
	pub server_binary: Option<PathBuf>,
}

/// Runs `leptoskit build`: a native project with generated hosts is built by
/// its platform build tools, and every other project through [`run`].
/// `optimize` overrides `bundle.optimize` for a [`run`] build.
pub fn command(
	release: bool,
	platform: Option<Platform>,
	optimize: Option<Optimize>,
	out: &str,
) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let platform = platform.unwrap_or(config.app.default_platform);
	config.ensure_platform_enabled(platform)?;
	if platform.is_native() {
		anyhow::ensure!(
			out == "dist",
			"native app outputs are managed by platform build tools; --out is only supported for \
			 web builds"
		);
		return crate::native::run(crate::native::NativeCommand::Build {
			target: crate::native::Target::from_platform(platform)?,
			release,
		});
	}
	if platform == Platform::Native
		&& (root.join("hosts/android").is_dir()
			|| config.native.targets == [leptoskit_config::NativeTarget::Harmonyos])
	{
		anyhow::ensure!(
			out == "dist",
			"native app outputs are managed by platform build tools; --out is only supported for \
			 web builds"
		);
		let target = crate::native::legacy_target(&root, &config);
		return crate::native::run(crate::native::NativeCommand::Build { target, release });
	}
	run(release, Some(platform), optimize, out).map(drop)
}

/// Builds the project. `optimize` overrides `bundle.optimize` for this
/// build.
pub fn run(
	release: bool,
	platform: Option<Platform>,
	optimize: Option<Optimize>,
	out: &str,
) -> Result<BuildOutput> {
	let (mut config, root) = LeptoskitConfig::load()?;
	if let Some(optimize) = optimize {
		config.bundle.optimize = optimize;
	}
	let platform = platform.unwrap_or(config.app.default_platform);
	config.ensure_platform_enabled(platform)?;
	anyhow::ensure!(
		!platform.is_native(),
		"use `leptoskit build --platform {platform}` for native platform builds"
	);
	let registry = resolve_plugins(&config)?;
	registry.generate(&root, platform, release)?;
	crate::l10n::sync(&root, &config)?;
	let root = Utf8PathBuf::from_path_buf(root)
		.map_err(|_| anyhow::anyhow!("project path is not UTF-8"))?;
	let out_dir = root.join(out);
	std::fs::create_dir_all(&out_dir).with_context(|| format!("could not create {out_dir}"))?;

	let (registry, context) = plugin_context(&config, platform, release)?;
	let mut rustflags = config.build.rustflags.clone();
	rustflags.extend(context.rustflags.iter().cloned());
	if !rustflags.is_empty() && std::env::var_os("RUSTFLAGS").is_some() {
		crate::ui::warning(
			"RUSTFLAGS is set, so cargo ignores build.rustflags and plugin rustflags",
		);
	}
	let options = CargoOptions {
		release,
		no_default_features: false,
		target_directory: None,
		features: context.cargo_features.clone(),
		rustflags,
		env: context.env.clone(),
		profile: None,
		config: Vec::new(),
	};

	let metadata = wasm::cargo_metadata(root.as_std_path())?;
	// A client-rendered app needs no server of its own: leptoskit's static
	// server serves `dist/`. Server rendering needs the app's server package.
	let server_package = metadata.package(&config.app.name);
	anyhow::ensure!(
		server_package.is_some() || !config.app.render.renders_on_server(),
		"app.name `{}` in leptoskit.toml must name the server package, but the cargo workspace \
		 has no such member",
		config.app.name
	);
	let client = wasm::detect_client_with(
		config.build.client_package.as_deref(),
		Some(&config.app.name),
		&metadata,
	)?;
	// One crate for the browser and native hosts declares both sides, and
	// each build selects its platform's feature.
	let portable = client
		.as_ref()
		.and_then(|client| metadata.package(&client.name))
		.is_some_and(|package| {
			package.declares_feature("web") && package.declares_feature("native")
		});
	let builds_client = matches!(platform, Platform::Web | Platform::Native) && client.is_some();
	// The render mode is a web concept: a native client is not hydrating
	// HTML, so only its server takes the mode's features.
	let client_package = client
		.as_ref()
		.filter(|_| builds_client && platform == Platform::Web)
		.and_then(|client| metadata.package(&client.name));
	let mode = match server_package {
		Some(server) => render::mode_features(config.app.render, server, client_package)?,
		None => render::ModeFeatures::default(),
	};
	let server_options = options.with_features(&mode.server);
	let client_options = client_cargo_options(
		&config,
		&root,
		platform,
		portable,
		client_package,
		options.with_features(&mode.client),
	)?;

	let server_binary = match server_package {
		Some(_) => compile_server(&root, &config.app.name, &server_options)?,
		None => None,
	};
	if let Some(binary) = &server_binary {
		BuiltServer {
			binary: binary.clone(),
		}
		.save(&metadata.target_directory, root.as_std_path())?;
	}

	let mut wasm_bound = false;
	let mut wasm_bundle = None;
	if let (true, Some(client)) = (builds_client, &client) {
		let wasm_path = wasm::compile_wasm(root.as_std_path(), client, &client_options)?;
		wasm::ensure_wasm_bindgen_compatible(&metadata.workspace_root.join("Cargo.lock"))?;

		let pkg_dir = out_dir.join("pkg").into_std_path_buf();
		let (bound, module) = bind_client(
			&wasm_path,
			&pkg_dir,
			client,
			release,
			&config.bundle,
			&metadata.target_directory,
		)?;
		wasm_bound = bound;
		wasm_bundle = Some(module);
	}
	if platform == Platform::Web {
		collect_assets(&root, &out_dir)?;
	}

	let mut post = leptoskit_plugin::PluginPostBuildContext {
		config: config.clone(),
		platform,
		release,
		output_dir: out_dir.as_std_path().to_path_buf(),
		wasm_bundle,
	};
	registry.after_build(&mut post)?;
	let mut sizes = BTreeMap::new();
	if platform == Platform::Web {
		if release && config.bundle.precompress {
			let mut cache =
				PrecompressCache::load(&metadata.target_directory, out_dir.as_std_path());
			sizes = compress::precompress(out_dir.as_std_path(), &mut cache)?;
			cache.save()?;
		} else {
			compress::remove_precompressed(out_dir.as_std_path())?;
		}
	}
	if platform != Platform::Server && client.is_none() {
		crate::ui::warning("no cdylib client crate found; built the server only");
	} else if wasm_bound {
		crate::ui::success(format_args!("wasm client bound into {out_dir}/pkg"));
	} else if platform != Platform::Server {
		crate::ui::skipped("wasm client unchanged; skipped wasm-bindgen");
	}
	crate::ui::success(format_args!(
		"built {platform} ({} render) into {out_dir}",
		config.app.render
	));
	if release && builds_client {
		report_client_bundle(&config.bundle, &out_dir, &sizes)?;
	}
	Ok(BuildOutput {
		out_dir,
		client,
		server_binary,
	})
}

/// The cargo options the wasm client compiles with: the render mode's
/// `client_features`, the web platform feature of a portable app, and the
/// size profile `bundle.optimize` (or `bundle.cargo_profile`) picks.
/// That profile only shapes the release client; the server always keeps the
/// project's `release` profile.
fn client_cargo_options(
	config: &LeptoskitConfig,
	root: &Utf8Path,
	platform: Platform,
	portable: bool,
	client_package: Option<&wasm::MetadataPackage>,
	client_features: CargoOptions,
) -> Result<CargoOptions> {
	let mut client_options = client_features;
	if portable && platform == Platform::Web {
		client_options = client_options.with_platform(platform)?;

		if config.app.render == leptoskit_config::RenderMode::Csr
			&& client_package.is_some_and(|package| package.declares_feature("csr"))
		{
			client_options = client_options.with_features(&["csr".to_owned()]);
		}

		client_options.target_directory =
			Some(root.join(".leptoskit/web/target").into_std_path_buf());
	}
	let mut client_options = match (&config.bundle.cargo_profile, config.bundle.optimize) {
		(Some(profile), _) => client_options.with_profile(profile, Vec::new()),
		(None, Optimize::Smallest) => {
			client_options.with_profile(wasm::SMALLEST_PROFILE, wasm::smallest_profile_config())
		}
		(None, Optimize::Fast) => client_options,
	};
	if config.bundle.split {
		client_options.rustflags.push(split::EMIT_RELOCS.to_owned());
	}
	Ok(client_options)
}

/// Prints a release client bundle's size per file, with the profile that
/// built it, and fails the build when it exceeds `bundle.budget_kib`.
fn report_client_bundle(
	bundle: &BundleConfig,
	out_dir: &Utf8Path,
	sizes: &BTreeMap<PathBuf, compress::Sizes>,
) -> Result<()> {
	let pkg_dir = out_dir.join("pkg").into_std_path_buf();
	let lazy = split::lazy_files(&pkg_dir)?;
	let report = BundleReport::measure(
		out_dir.as_std_path(),
		&pkg_dir,
		|path| lazy.contains(path),
		sizes,
	)?;
	let budget = bundle.budget_kib;
	let profile = match &bundle.cargo_profile {
		Some(profile) => format!("cargo_profile = \"{profile}\""),
		None => format!("optimize = \"{}\"", bundle.optimize),
	};
	anstream::println!(
		"\n{BOLD}client bundle{BOLD:#} {DIM}({profile}){DIM:#}\n{}",
		report.render(budget)
	);
	report.check_budget(budget)
}

/// The phases after cargo: split (with `bundle.split`), bind, and, in
/// release, optimize. They are skipped together when their fingerprint is
/// current and their outputs are on disk. Returns whether they ran, and the
/// bound main module.
///
/// # Errors
///
/// Fails before binding when the client has `#[lazy]` code but
/// `bundle.split` is off: the page would import a chunk loader that does not
/// exist.
pub fn bind_client(
	wasm_path: &Path,
	pkg_dir: &Path,
	client: &ClientInfo,
	release: bool,
	bundle: &BundleConfig,
	target_dir: &Path,
) -> Result<(bool, PathBuf)> {
	let wasm = std::fs::read(wasm_path)
		.with_context(|| format!("could not read {}", wasm_path.display()))?;
	let has_lazy_code = split::uses_lazy_loading(&wasm)?;
	anyhow::ensure!(
		bundle.split || !has_lazy_code,
		"`{}` has #[lazy] code (a lazy function, lazy route, or #[island(lazy)]), which loads \
		 from chunks only code splitting produces; set `split = true` under [bundle] in \
		 leptoskit.toml",
		client.name
	);
	let splits = bundle.split && has_lazy_code;

	if splits {
		split::ensure_relocs_reach_cargo(&|variable| std::env::var_os(variable))?;
	}
	// wasm-opt never runs in dev: it is the slowest phase and the size win
	// is irrelevant while iterating.
	let optimizes = release && bundle.wasm_opt;
	let wasm_opt_version = optimizes.then(wasm::wasm_opt_version).transpose()?;
	let fingerprint = wasm::bindgen_fingerprint(
		&hash_bytes(&wasm),
		release,
		pkg_dir,
		wasm_opt_version.as_deref(),
		splits,
	);
	let [glue, module] = wasm::bindgen_outputs(pkg_dir, &client.lib_name);
	let chunks_present = || {
		pkg_dir.join(split::MANIFEST).is_file()
			&& split::lazy_files(pkg_dir)
				.is_ok_and(|chunks| chunks.iter().all(|chunk| chunk.is_file()))
	};
	let outputs_present = glue.is_file() && module.is_file() && (!splits || chunks_present());
	let mut cache = BuildCache::load(target_dir);
	if outputs_present && cache.is_current(&fingerprint) {
		return Ok((false, module));
	}

	// A fresh pkg/, so the chunks and snippets of an earlier build never
	// linger next to this one's.
	if pkg_dir.exists() {
		std::fs::remove_dir_all(pkg_dir)
			.with_context(|| format!("could not clear {}", pkg_dir.display()))?;
	}
	std::fs::create_dir_all(pkg_dir)
		.with_context(|| format!("could not create {}", pkg_dir.display()))?;
	let (bindgen_input, chunks) = if splits {
		let split = split::split(&wasm, wasm_path, pkg_dir, &client.lib_name)?;
		(split.main, split.chunks)
	} else {
		(wasm_path.to_path_buf(), Vec::new())
	};
	drop(wasm);
	wasm::bindgen(&bindgen_input, pkg_dir, &client.lib_name, release, splits)?;
	if optimizes {
		use rayon::prelude::*;

		std::iter::once(&module)
			.chain(&chunks)
			.collect::<Vec<_>>()
			.into_par_iter()
			.try_for_each(|path| wasm::optimize(path))?;
	}
	cache.record(&fingerprint);
	cache.save().context("could not save the build cache")?;
	Ok((true, module))
}

/// The cargo options a server-only build uses: `build.features`, the leptos
/// plugin features, and plugin rustflags and environment.
pub(crate) fn server_options(config: &LeptoskitConfig, release: bool) -> Result<CargoOptions> {
	let (_registry, context) = plugin_context(config, Platform::Server, release)?;
	let mut rustflags = config.build.rustflags.clone();
	rustflags.extend(context.rustflags.iter().cloned());
	Ok(CargoOptions {
		release,
		no_default_features: false,
		target_directory: None,
		features: context.cargo_features.clone(),
		rustflags,
		env: context.env.clone(),
		profile: None,
		config: Vec::new(),
	})
}

/// The router declaration a configuration activates: the explicit
/// `[[plugins]]` entry naming `router` when present, otherwise the legacy
/// `[router]` table expressed as one.
pub(crate) fn router_spec(config: &LeptoskitConfig) -> Option<leptoskit_config::PluginSpec> {
	if let Some(spec) = config
		.plugins
		.iter()
		.find(|spec| !spec.leptos && spec.name == "router")
	{
		return Some(spec.clone());
	}
	let router = config.router.as_ref()?;
	let mut spec = leptoskit_config::PluginSpec {
		name: "router".to_owned(),
		..Default::default()
	};
	if router.dir != leptoskit_config::RouterConfig::default().dir {
		spec.options.insert(
			"dir".to_owned(),
			toml::Value::String(router.dir.display().to_string()),
		);
	}
	Some(spec)
}

/// Runs plugin generation and returns every file the plugins wrote, for the
/// dev watcher to exclude from re-triggering builds.
pub(crate) fn generate_outputs(root: &Path, config: &LeptoskitConfig) -> Result<Vec<PathBuf>> {
	let registry = resolve_plugins(config)?;
	registry
		.generate(root, Platform::Web, false)
		.map_err(|error| anyhow::anyhow!(error))
}

/// Resolves the plugin registry, refusing declarations the tooling cannot
/// load: silently dropping one would build something other than what the
/// configuration asks for. The router plugin joins from its declaration or
/// the legacy `[router]` table.
pub(crate) fn resolve_plugins(
	config: &LeptoskitConfig,
) -> Result<leptoskit_plugin::PluginRegistry> {
	let (mut registry, unresolved) = leptoskit_plugin::PluginRegistry::resolve(config)?;
	let mut unknown = Vec::new();
	for plugin in &unresolved {
		if plugin.spec.name == "router" {
			continue;
		}
		// A `command` (or a `leptoskit-plugin-<name>` on PATH) makes the
		// plugin a separate program speaking the process protocol.
		let command = plugin
			.spec
			.command
			.clone()
			.unwrap_or_else(|| leptoskit_plugin::default_command(&plugin.spec.name));
		if leptoskit_plugin::command_exists(&command) {
			let process = leptoskit_plugin::ProcessPlugin::probe(
				&plugin.spec.name,
				&command,
				leptoskit_plugin::toml_options(&plugin.spec.options),
			)
			.map_err(|error| anyhow::anyhow!(error))?;
			registry.register(process);
		} else {
			unknown.push(plugin);
		}
	}
	let unknown = unknown;
	if !unknown.is_empty() {
		let names: Vec<&str> = unknown
			.iter()
			.map(|plugin| plugin.spec.name.as_str())
			.collect();
		anyhow::bail!(
			"unknown plugin(s) in leptoskit.toml: {}; the built-ins are {}, and `router`; leptos \
			 plugins (cargo features of the app) need `leptos = true`",
			names.join(", "),
			leptoskit_plugin::Builtins::NAMES.join(", ")
		);
	}
	if let Some(spec) = router_spec(config) {
		let plugin = leptoskit_plugin_router::RouterPlugin::default();
		plugin
			.configure(leptoskit_plugin::toml_options(&spec.options))
			.map_err(|error| anyhow::anyhow!(error))?;
		registry.register(plugin);
	}
	Ok(registry)
}

/// Resolves the plugins and runs their `before_serve` hooks over the server
/// about to start.
///
/// # Errors
///
/// Returns an error for an undeclarable plugin or a hook that fails.
pub(crate) fn serve_plugins(config: &LeptoskitConfig) -> Result<()> {
	let registry = resolve_plugins(config)?;
	let context = leptoskit_plugin::PluginBuildContext::new(config.clone(), Platform::Web, false);
	registry.before_serve(&context)?;
	Ok(())
}

/// Resolves the plugins and runs their `before_build` hooks over a context
/// seeded with `build.features` and the leptos plugin features.
///
/// # Errors
///
/// Returns an error when a declared plugin is neither a built-in nor a leptos
/// plugin: silently dropping it would build something other than what the
/// configuration asks for.
pub(crate) fn plugin_context(
	config: &LeptoskitConfig,
	platform: Platform,
	release: bool,
) -> Result<(
	leptoskit_plugin::PluginRegistry,
	leptoskit_plugin::PluginBuildContext,
)> {
	let registry = resolve_plugins(config)?;
	registry.ensure_supported(platform)?;
	let mut context = leptoskit_plugin::PluginBuildContext::new(config.clone(), platform, release);
	for feature in &config.build.features {
		context.add_feature(feature.clone());
	}
	// Leptos plugins are forwarded as cargo features.
	for spec in &config.plugins {
		if spec.leptos {
			context.add_feature(spec.name.clone());
		}
	}
	registry.before_build(&mut context)?;
	Ok((registry, context))
}

/// Runs cargo build for the server package and returns its server
/// executable, read from cargo's JSON artifact messages so renamed binaries
/// and custom target directories resolve exactly. Diagnostics still render
/// to the terminal as usual.
pub(crate) fn compile_server(
	root: &Utf8PathBuf,
	package: &str,
	options: &CargoOptions,
) -> Result<Option<PathBuf>> {
	let mut child = wasm::cargo_build_command(root.as_std_path(), package, options)
		.arg("--message-format=json-render-diagnostics")
		.stdout(Stdio::piped())
		.spawn()
		.context("could not run cargo")?;
	let messages = child
		.stdout
		.take()
		.context("cargo's output was not captured")?;
	let binaries = server::binaries_in_messages(BufReader::new(messages))?;
	let status = child.wait().context("could not wait for cargo")?;
	anyhow::ensure!(status.success(), "cargo build failed");
	Ok(server::pick_server(package, binaries))
}

/// Copies the public asset tree into the output, hashing in parallel and
/// copying only files whose content differs from the existing copy.
fn collect_assets(root: &Utf8PathBuf, out_dir: &Utf8PathBuf) -> Result<()> {
	for directory in ["web", "public", "assets"] {
		let destination = if directory == "assets" {
			out_dir.join("assets")
		} else {
			out_dir.clone()
		};
		copy_assets(&root.join(directory), &destination)?;
	}

	Ok(())
}

fn copy_assets(public: &Utf8PathBuf, out_dir: &Utf8PathBuf) -> Result<()> {
	use rayon::prelude::*;

	if !public.is_dir() {
		return Ok(());
	}
	let files: Vec<PathBuf> = walkdir::WalkDir::new(public.as_std_path())
		.into_iter()
		.filter_map(Result::ok)
		.filter(|entry| entry.file_type().is_file())
		.map(walkdir::DirEntry::into_path)
		.collect();
	let fingerprints: Vec<(PathBuf, String)> = files
		.into_par_iter()
		.map(|path| {
			let content = std::fs::read(&path)
				.with_context(|| format!("could not read {}", path.display()))?;
			Ok((path, hash_bytes(&content)))
		})
		.collect::<Result<_>>()?;
	for (path, hash) in fingerprints {
		let relative = path
			.strip_prefix(public.as_std_path())
			.context("asset outside the public tree")?;
		let target = out_dir.as_std_path().join(relative);
		if let Some(parent) = target.parent() {
			std::fs::create_dir_all(parent)?;
		}
		let current = std::fs::read(&target)
			.ok()
			.map(|content| hash_bytes(&content));
		if current.as_deref() != Some(hash.as_str()) {
			std::fs::copy(&path, &target).with_context(|| format!("copying {}", path.display()))?;
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_legacy_router_table_becomes_a_plugin_declaration() {
		let default = LeptoskitConfig {
			router: Some(leptoskit_config::RouterConfig::default()),
			..Default::default()
		};
		let spec = router_spec(&default).expect("the legacy table activates the plugin");
		assert_eq!(spec.name, "router");
		assert!(spec.options.is_empty(), "the default directory is implied");

		let custom = LeptoskitConfig {
			router: Some(leptoskit_config::RouterConfig {
				dir: PathBuf::from("app/ui"),
			}),
			..Default::default()
		};
		let spec = router_spec(&custom).expect("a custom directory carries over");
		assert_eq!(spec.options["dir"], "app/ui".into());

		assert!(router_spec(&LeptoskitConfig::default()).is_none());
	}
}
