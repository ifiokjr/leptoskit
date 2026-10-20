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
//!   already matches the copy in the output.

use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;

use crate::cache::BuildCache;
use crate::cache::hash_bytes;
use crate::wasm::CargoOptions;
use crate::wasm::{self};

/// Builds the project.
pub fn run(release: bool, platform: Option<Platform>, out: &str) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let platform = platform.unwrap_or(config.app.default_platform);
	let root = Utf8PathBuf::from_path_buf(root)
		.map_err(|_| anyhow::anyhow!("project path is not UTF-8"))?;
	let out_dir = root.join(out);
	std::fs::create_dir_all(&out_dir).with_context(|| format!("could not create {out_dir}"))?;

	let (registry, context) = plugin_context(&config, platform, release)?;
	let mut rustflags = config.build.rustflags.clone();
	rustflags.extend(context.rustflags.iter().cloned());
	if !rustflags.is_empty() && std::env::var_os("RUSTFLAGS").is_some() {
		eprintln!(
			"warning: RUSTFLAGS is set, so cargo ignores build.rustflags and plugin rustflags"
		);
	}
	let options = CargoOptions {
		release,
		features: context.cargo_features.clone(),
		rustflags,
		env: context.env.clone(),
	};

	let metadata = wasm::cargo_metadata(root.as_std_path())?;
	anyhow::ensure!(
		metadata.package(&config.app.name).is_some(),
		"app.name `{}` in leptoskit.toml must name the server package, but the cargo workspace \
		 has no such member",
		config.app.name
	);
	let client = wasm::detect_client_with(config.build.client_package.as_deref(), &metadata)?;

	compile_server(&root, &config.app.name, &options)?;

	let mut wasm_bound = false;
	let mut wasm_bundle = None;
	if let (Platform::Web | Platform::Native, Some(client)) = (platform, &client) {
		let wasm_path = wasm::compile_wasm(
			root.as_std_path(),
			&metadata.target_directory,
			client,
			&options,
		)?;
		wasm::ensure_wasm_bindgen_compatible(&metadata.workspace_root.join("Cargo.lock"))?;

		let pkg_dir = out_dir.join("pkg").into_std_path_buf();
		// wasm-opt never runs in dev: it is the slowest phase and the size
		// win is irrelevant while iterating.
		let optimizes = release && config.build.wasm_opt;
		let wasm_opt_version = optimizes.then(wasm::wasm_opt_version).transpose()?;
		let fingerprint = wasm::bindgen_fingerprint(
			&wasm::hash_file(&wasm_path)?,
			release,
			&pkg_dir,
			wasm_opt_version.as_deref(),
		);
		let outputs = wasm::bindgen_outputs(&pkg_dir, &client.lib_name);
		let outputs_present = outputs.iter().all(|path| path.is_file());
		let [_glue, module] = outputs;

		let mut cache = BuildCache::load(&metadata.target_directory);
		if !outputs_present || !cache.is_current(&fingerprint) {
			std::fs::create_dir_all(&pkg_dir)
				.with_context(|| format!("could not create {}", pkg_dir.display()))?;
			wasm::bindgen(&wasm_path, &pkg_dir, release)?;
			if optimizes {
				wasm::optimize(&module)?;
			}
			cache.record(&fingerprint);
			cache.save().context("could not save the build cache")?;
			wasm_bound = true;
		}
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
	if platform != Platform::Server && client.is_none() {
		println!("no cdylib client crate found; built the server only");
	} else if wasm_bound {
		println!("wasm client bound into {out_dir}/pkg");
	} else if platform != Platform::Server {
		println!("wasm client unchanged; skipped wasm-bindgen");
	}
	println!("built {platform} into {out_dir}");
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
fn plugin_context(
	config: &LeptoskitConfig,
	platform: Platform,
	release: bool,
) -> Result<(
	leptoskit_plugin::PluginRegistry,
	leptoskit_plugin::PluginBuildContext,
)> {
	let (registry, unresolved) = leptoskit_plugin::PluginRegistry::resolve(config)?;
	if !unresolved.is_empty() {
		let names: Vec<&str> = unresolved
			.iter()
			.map(|plugin| plugin.spec.name.as_str())
			.collect();
		anyhow::bail!(
			"unknown plugin(s) in leptoskit.toml: {}; the built-ins are {}, and leptos plugins \
			 (cargo features of the app) need `leptos = true`",
			names.join(", "),
			leptoskit_plugin::Builtins::NAMES.join(", ")
		);
	}
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

/// Runs cargo build for the server binary. Pure: runs cargo, returns nothing.
fn compile_server(root: &Utf8PathBuf, package: &str, options: &CargoOptions) -> Result<()> {
	let status = wasm::cargo_build_command(root.as_std_path(), package, options)
		.status()
		.context("could not run cargo")?;
	anyhow::ensure!(status.success(), "cargo build failed");
	Ok(())
}

/// Copies the public asset tree into the output, hashing in parallel and
/// copying only files whose content differs from the existing copy.
fn collect_assets(root: &Utf8PathBuf, out_dir: &Utf8PathBuf) -> Result<()> {
	use rayon::prelude::*;

	let public = root.join("public");
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
