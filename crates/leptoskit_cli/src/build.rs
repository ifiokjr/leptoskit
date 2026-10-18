//! `leptoskit build`: cached, parallel builds for every platform.
//!
//! The pipeline mirrors what cargo-leptos does — compile the server, compile
//! the wasm client, run wasm-bindgen, optionally optimize, lay out assets —
//! with two differences that make it fast:
//!
//! - the server and the wasm client compile **in parallel** on separate
//!   threads, so the wall time is the slower of the two rather than the sum;
//! - every phase is content-fingerprinted through [`crate::cache`], so a
//!   rebuild after a one-line change runs only the phases whose inputs moved.
//!   An unchanged wasm skips wasm-bindgen entirely; `wasm-opt` only ever runs
//!   in release.

use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;

use crate::cache::BuildCache;
use crate::cache::Fingerprint;
use crate::cache::hash_bytes;
use crate::wasm::WasmOptions;
use crate::wasm::{self};

/// Builds the project.
pub fn run(release: bool, platform: Option<Platform>, out: &str) -> Result<()> {
	let (config, root) = load_config()?;
	let platform = platform.unwrap_or(config.app.default_platform);
	let root = Utf8PathBuf::from_path_buf(root)
		.map_err(|_| anyhow::anyhow!("project path is not UTF-8"))?;
	let out_dir = root.join(out);
	std::fs::create_dir_all(&out_dir).with_context(|| format!("could not create {out_dir}"))?;

	// Plugins run before the pipeline: they may add features or environment.
	let (registry, _unresolved) = leptoskit_plugin::PluginRegistry::resolve(&config)?;
	let mut context = leptoskit_plugin::PluginBuildContext::new(config.clone(), platform, release);
	registry.before_build(&mut context)?;
	// Leptos plugins are forwarded as cargo features.
	for spec in &config.plugins {
		if spec.leptos {
			context.add_feature(spec.name.clone());
		}
	}

	let client_package =
		wasm::detect_client_package(root.as_std_path(), config.build.client_package.as_deref())?;
	let mut cache = BuildCache::load(root.as_std_path());

	let mut wasm_bound = false;
	match platform {
		Platform::Server => {
			let fingerprint = server_fingerprint(&root, release, &context)?;
			if !cache.is_current(&fingerprint) {
				compile_server(&root, release, &context)?;
				cache.record(&fingerprint);
			}
		}
		Platform::Web | Platform::Native => {
			let server_fp = server_fingerprint(&root, release, &context)?;
			let need_server = !cache.is_current(&server_fp);

			let mut wasm_fp = None;
			if let Some(package) = &client_package {
				let options = WasmOptions {
					package: package.clone(),
					release,
					features: context.cargo_features.clone(),
					rustflags: config.build.rustflags.clone(),
					wasm_opt: config.build.wasm_opt,
					out_dir: out_dir.clone().into_std_path_buf(),
				};
				let fingerprint = wasm::compile_fingerprint(root.as_std_path(), &options)?;
				wasm_fp = Some((fingerprint, options));
			}

			// The two compiles are pure, so they overlap freely; the wall
			// time is the slower of the two, not the sum.
			let need_wasm = wasm_fp
				.as_ref()
				.is_some_and(|(fingerprint, _)| !cache.is_current(fingerprint));
			let client_options = || {
				wasm_fp
					.as_ref()
					.expect("need_wasm implies the client options exist")
			};
			let wasm_result = match (need_server, need_wasm) {
				(true, true) => {
					let (fingerprint, options) = client_options();
					let result = wasm::join_parallel(
						{
							let root = root.clone();
							let context = context.clone();
							move || compile_server(&root, release, &context)
						},
						{
							let root = root.clone();
							let options = options.clone();
							move || wasm::compile_wasm(root.as_std_path(), &options)
						},
					);
					let (_server, wasm) = result?;
					Some((Ok(wasm), fingerprint.clone()))
				}
				(true, false) => {
					compile_server(&root, release, &context)?;
					None
				}
				(false, true) => {
					let (fingerprint, options) = client_options();
					Some((
						wasm::compile_wasm(root.as_std_path(), options),
						fingerprint.clone(),
					))
				}
				(false, false) => None,
			};

			if need_server {
				cache.record(&server_fp);
			}

			if let Some((wasm_result, compile_fp)) = wasm_result {
				let wasm_path = wasm_result?;
				cache.record(&compile_fp);

				// wasm-bindgen is keyed on the wasm content: an unchanged
				// compile skips it entirely.
				let wasm_hash = wasm::hash_file(&wasm_path)?;
				let bindgen_fp = wasm::bindgen_fingerprint(&wasm_hash, release);
				if !cache.is_current(&bindgen_fp) {
					let pkg_dir = out_dir.join("pkg");
					std::fs::create_dir_all(&pkg_dir)
						.with_context(|| format!("could not create {pkg_dir}"))?;
					wasm::bindgen(&wasm_path, pkg_dir.as_std_path(), release)?;
					cache.record(&bindgen_fp);
					wasm_bound = true;
				}

				// wasm-opt never runs in dev: it is the slowest phase and the
				// size win is irrelevant while iterating.
				if release && config.build.wasm_opt {
					let package = client_package
						.as_deref()
						.expect("the wasm phases only run with a client package");
					let optimized_wasm = out_dir
						.join("pkg")
						.join(format!("{}_bg.wasm", package.replace('-', "_")));
					if optimized_wasm.is_file() {
						let optimized_hash = wasm::hash_file(optimized_wasm.as_std_path())?;
						let opt_fp = wasm::optimize_fingerprint(&optimized_hash);
						if !cache.is_current(&opt_fp) {
							wasm::optimize(optimized_wasm.as_std_path())?;
							cache.record(&opt_fp);
						}
					}
				}
			}
		}
	}
	if platform == Platform::Web {
		collect_assets(&root, &out_dir)?;
	}
	cache.save().context("could not save the build cache")?;

	let mut post = leptoskit_plugin::PluginPostBuildContext {
		config: config.clone(),
		platform,
		release,
		output_dir: out_dir.as_std_path().to_path_buf(),
		wasm_bundle: client_package.as_ref().map(|package| {
			out_dir
				.join("pkg")
				.join(format!("{}_bg.wasm", package.replace('-', "_")))
				.into_std_path_buf()
		}),
	};
	registry.after_build(&mut post)?;
	if platform == Platform::Web && client_package.is_none() {
		println!("no cdylib client crate found; built the server only");
	} else if wasm_bound {
		println!("wasm client bound into {out_dir}/pkg");
	}
	println!("built {platform} into {out_dir}");
	Ok(())
}

/// Loads the configuration from the current directory upward.
fn load_config() -> Result<(LeptoskitConfig, PathBuf)> {
	let (config, root) = LeptoskitConfig::load().context("no leptoskit project found")?;
	Ok((config, root))
}

/// The server phase's fingerprint.
fn server_fingerprint(
	root: &Utf8PathBuf,
	release: bool,
	context: &leptoskit_plugin::PluginBuildContext,
) -> Result<Fingerprint> {
	Ok(Fingerprint::new("cargo-server", cargo_version()?)
		.with_file("manifest", root.join("Cargo.toml").as_std_path())
		.with_input("release", if release { "1" } else { "0" }.to_owned())
		.with_input("features", context.cargo_features.join(",")))
}

/// Runs cargo build for the server binary. Pure: runs cargo, returns nothing.
fn compile_server(
	root: &Utf8PathBuf,
	release: bool,
	context: &leptoskit_plugin::PluginBuildContext,
) -> Result<()> {
	let mut command = Command::new("cargo");
	command
		.arg("build")
		.arg("--package")
		.arg(&context.config.app.name)
		.current_dir(root.as_std_path());
	if release {
		command.arg("--release");
	}
	if !context.cargo_features.is_empty() {
		command
			.arg("--features")
			.arg(context.cargo_features.join(","));
	}
	let status = command.status().context("could not run cargo")?;
	anyhow::ensure!(status.success(), "cargo build failed");
	Ok(())
}

/// Hashes every file under a directory, sorted by path.
pub fn directory_digest(dir: &std::path::Path) -> String {
	let mut entries: Vec<(String, String)> = walkdir::WalkDir::new(dir)
		.sort_by_file_name()
		.into_iter()
		.filter_map(Result::ok)
		.filter(|entry| entry.file_type().is_file())
		.filter_map(|entry| {
			let path = entry.path().strip_prefix(dir).ok()?;
			let content = std::fs::read(entry.path()).ok()?;
			Some((
				path.to_string_lossy().replace('\\', "/"),
				hash_bytes(&content),
			))
		})
		.collect();
	entries.sort();
	hash_bytes(&serde_json::to_vec(&entries).unwrap_or_default())
}

/// Copies the public asset tree into the output, hashing in parallel.
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
			let hash = match std::fs::read(&path) {
				Ok(content) => hash_bytes(&content),
				Err(_) => "missing".to_owned(),
			};
			(path, hash)
		})
		.collect();
	for (path, hash) in fingerprints {
		let relative = path
			.strip_prefix(public.as_std_path())
			.context("asset outside the public tree")?;
		let target = out_dir.as_std_path().join(relative);
		if let Some(parent) = target.parent() {
			std::fs::create_dir_all(parent)?;
		}
		let needs_copy = std::fs::read(&target)
			.map(|existing| hash_bytes(&existing) != hash)
			.unwrap_or(true);
		if needs_copy {
			std::fs::copy(&path, &target).with_context(|| format!("copying {}", path.display()))?;
		}
	}
	Ok(())
}

/// The cargo version string, part of every tool fingerprint.
fn cargo_version() -> Result<String> {
	let output = Command::new("cargo")
		.arg("--version")
		.output()
		.context("could not run cargo")?;
	Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
