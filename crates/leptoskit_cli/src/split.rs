//! Code splitting with wasm-split.
//!
//! leptos marks code that can load later with `#[lazy]`: lazy functions,
//! lazy routes, and `#[island(lazy)]`. Each compiles to a call through an
//! import from [`PLACEHOLDER_MODULE`], which only wasm-split can resolve.
//! With `bundle.split`, leptoskit runs wasm-split over the compiled client in
//! process, before wasm-bindgen:
//!
//! 1. the client links with `--emit-relocs`, so wasm-split can see which
//!    functions each lazy entry point reaches;
//! 2. wasm-split moves that code into chunk modules in `pkg/`, points the
//!    placeholder imports at the [`LINK_MODULE`] loader it writes beside them,
//!    and leaves a smaller main module;
//! 3. leptoskit writes [`MANIFEST`], which leptos' `HydrationScripts` reads on
//!    the server, so a page can preload the chunks of the lazy functions the
//!    server called while rendering it;
//! 4. wasm-bindgen binds the main module, keeping the exports the chunks link
//!    against.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;

/// The import module every `#[lazy]` call compiles against. wasm-split
/// rewrites it to [`LINK_MODULE`]; unsplit, the bound client would import a
/// module no browser can resolve.
pub const PLACEHOLDER_MODULE: &str = "__wasm_split_placeholder__";

/// The chunk loader wasm-split writes beside the chunks. The name is fixed:
/// leptos' `HydrationScripts` looks for it.
pub const LINK_MODULE: &str = "__wasm_split.______________________.js";

/// The chunk manifest leptos reads from the site's `pkg/` directory: each
/// split point mapped to the chunk files (by stem) it needs.
pub const MANIFEST: &str = "__wasm_split_manifest.json";

/// The linker flag that keeps relocations in the compiled wasm, which is
/// what wasm-split reads to find the code reachable from each split point.
pub const EMIT_RELOCS: &str = "-Clink-args=--emit-relocs";

/// Fails when the environment overrides the rustflags leptoskit passes
/// through `--config`, which would drop [`EMIT_RELOCS`]: cargo ignores them
/// while `CARGO_ENCODED_RUSTFLAGS` or `RUSTFLAGS` is set, unless that
/// variable carries the flag itself. `variable` reads the environment.
pub fn ensure_relocs_reach_cargo(variable: &dyn Fn(&str) -> Option<OsString>) -> Result<()> {
	for name in ["CARGO_ENCODED_RUSTFLAGS", "RUSTFLAGS"] {
		if let Some(flags) = variable(name) {
			anyhow::ensure!(
				flags.to_string_lossy().contains("--emit-relocs"),
				"bundle.split links the client with `{EMIT_RELOCS}`, but cargo ignores \
				 leptoskit's flags while {name} is set; unset {name} or add the flag to it",
			);

			return Ok(());
		}
	}

	Ok(())
}

/// What splitting produced.
#[derive(Debug, Clone)]
pub struct SplitOutput {
	/// The main module, ready for wasm-bindgen.
	pub main: PathBuf,
	/// The chunk modules, already in `pkg/`.
	pub chunks: Vec<PathBuf>,
}

/// Splits `input`, the compiled client read from `compiled`, into chunks in
/// `pkg_dir`.
///
/// The main module is written next to the compiled wasm, as
/// `<lib_name>_split.wasm`, so wasm-bindgen can bind it into `pkg_dir` under
/// the client's usual name.
pub fn split(input: &[u8], compiled: &Path, pkg_dir: &Path, lib_name: &str) -> Result<SplitOutput> {
	let main = compiled.with_file_name(format!("{lib_name}_split.wasm"));
	let main_module = format!("./{lib_name}.js");
	let link_name = format!("./{LINK_MODULE}");
	let mut options = wasm_split_cli_support::Options::new(input);
	options.output_dir = pkg_dir;
	options.main_out_path = &main;
	options.main_module = &main_module;
	options.link_name = &link_name;

	let split = wasm_split_cli_support::transform(options).map_err(|error| {
		anyhow::anyhow!(
			"wasm-split could not split {}: {error:?}\nbundle.split needs the client linked with \
			 `{EMIT_RELOCS}`, which leptoskit adds unless RUSTFLAGS overrides it",
			compiled.display()
		)
	})?;
	let manifest: BTreeMap<_, _> = split.prefetch_map.into_iter().collect();
	let manifest_path = pkg_dir.join(MANIFEST);
	std::fs::write(
		&manifest_path,
		serde_json::to_vec_pretty(&manifest).context("could not encode the split manifest")?,
	)
	.with_context(|| format!("could not write {}", manifest_path.display()))?;
	Ok(SplitOutput {
		main,
		chunks: split.split_modules,
	})
}

/// Whether the compiled client imports from [`PLACEHOLDER_MODULE`]: it was
/// built with `#[lazy]` code, which only runs once wasm-split has produced
/// the chunks and the loader.
pub fn uses_lazy_loading(wasm: &[u8]) -> Result<bool> {
	for payload in wasmparser::Parser::new(0).parse_all(wasm) {
		if let wasmparser::Payload::ImportSection(imports) =
			payload.context("could not parse the compiled wasm")?
		{
			for import in imports.into_imports() {
				let import = import.context("could not parse a wasm import")?;

				if import.module == PLACEHOLDER_MODULE {
					return Ok(true);
				}
			}

			return Ok(false);
		}
	}

	Ok(false)
}

/// The chunk files the manifest in `pkg_dir` lists, which load on demand
/// rather than with the page. Empty when the build was not split.
pub fn lazy_files(pkg_dir: &Path) -> Result<BTreeSet<PathBuf>> {
	let path = pkg_dir.join(MANIFEST);

	let json = match std::fs::read(&path) {
		Ok(json) => json,

		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
			return Ok(BTreeSet::new());
		}

		Err(error) => {
			return Err(error).with_context(|| format!("could not read {}", path.display()));
		}
	};

	let manifest: BTreeMap<String, Vec<String>> = serde_json::from_slice(&json)
		.with_context(|| format!("could not parse {}", path.display()))?;
	Ok(manifest
		.into_values()
		.flatten()
		.map(|stem| pkg_dir.join(format!("{stem}.wasm")))
		.collect())
}
