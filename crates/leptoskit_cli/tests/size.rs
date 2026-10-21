//! Bundle size tests: the size profile, precompression, and the report and
//! budget release builds print.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_cli::compress;
use leptoskit_cli::compress::PrecompressCache;
use leptoskit_cli::compress::Sizes;
use leptoskit_cli::report::BundleReport;
use leptoskit_cli::wasm;
use leptoskit_cli::wasm::CargoOptions;

struct Scratch {
	path: PathBuf,
}

impl Scratch {
	fn new(tag: &str) -> Self {
		static COUNTER: AtomicU64 = AtomicU64::new(0);
		let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
		let path = std::env::temp_dir().join(format!(
			"leptoskit_size_test_{tag}_{}_{}",
			std::process::id(),
			unique
		));
		std::fs::create_dir_all(&path).expect("scratch");
		let path = std::fs::canonicalize(&path).expect("canonical scratch");
		Self { path }
	}
}

impl Drop for Scratch {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.path);
	}
}

/// Text that compresses well, so its variants are always kept.
fn compressible_text() -> String {
	"leptoskit ships small bundles. ".repeat(200)
}

fn unbrotli(bytes: &[u8]) -> Vec<u8> {
	let mut out = Vec::new();
	brotli::Decompressor::new(bytes, 4096)
		.read_to_end(&mut out)
		.expect("valid brotli");
	out
}

fn ungzip(bytes: &[u8]) -> Vec<u8> {
	let mut out = Vec::new();
	flate2::read::GzDecoder::new(bytes)
		.read_to_end(&mut out)
		.expect("valid gzip");
	out
}

#[test]
fn only_text_and_wasm_are_compressible() {
	for name in [
		"app.wasm",
		"app.js",
		"style.css",
		"index.html",
		"logo.svg",
		"data.json",
	] {
		assert!(compress::is_compressible(Path::new(name)), "{name}");
	}
	for name in [
		"photo.png",
		"font.woff2",
		"archive.tar.gz",
		"app.js.br",
		"README",
	] {
		assert!(!compress::is_compressible(Path::new(name)), "{name}");
	}
}

#[test]
fn precompress_writes_decodable_brotli_and_gzip_beside_each_file() {
	let scratch = Scratch::new("precompress");
	let pkg = scratch.path.join("pkg");
	std::fs::create_dir_all(&pkg).expect("pkg");
	let text = compressible_text();
	std::fs::write(pkg.join("app.js"), &text).expect("write");
	std::fs::write(scratch.path.join("photo.png"), &text).expect("write");

	let sizes = compress::precompress(&scratch.path, &mut PrecompressCache::default())
		.expect("precompress");

	let brotli = std::fs::read(pkg.join("app.js.br")).expect("brotli variant");
	let gzip = std::fs::read(pkg.join("app.js.gz")).expect("gzip variant");
	assert_eq!(unbrotli(&brotli), text.as_bytes());
	assert_eq!(ungzip(&gzip), text.as_bytes());
	assert!(brotli.len() < text.len() && gzip.len() < text.len());
	assert_eq!(
		sizes[&pkg.join("app.js")],
		Sizes {
			raw: text.len() as u64,
			gzip: gzip.len() as u64,
			brotli: brotli.len() as u64,
		},
		"precompression reports what it wrote, for the bundle report to reuse"
	);
	assert!(
		!scratch.path.join("photo.png.br").exists(),
		"images are already compressed"
	);
}

#[test]
fn precompression_skips_files_whose_content_and_variants_are_unchanged() {
	let scratch = Scratch::new("cached");
	let out = scratch.path.join("dist");
	std::fs::create_dir_all(&out).expect("dist");
	let page = out.join("app.js");
	std::fs::write(&page, compressible_text()).expect("write");
	let target = scratch.path.join("target");

	let mut cache = PrecompressCache::load(&target);
	let first = compress::precompress(&out, &mut cache).expect("precompress");
	cache.save().expect("save");

	// A same-length stand-in for the brotli copy survives the next build only
	// if precompression skipped the unchanged file.
	let brotli = out.join("app.js.br");
	let stand_in = vec![b'x'; std::fs::metadata(&brotli).expect("br").len() as usize];
	std::fs::write(&brotli, &stand_in).expect("stand in");
	let mut cache = PrecompressCache::load(&target);
	let second = compress::precompress(&out, &mut cache).expect("precompress");
	assert_eq!(second, first, "unchanged files report their recorded sizes");
	assert_eq!(std::fs::read(&brotli).expect("br"), stand_in);

	// A missing variant, or new content, is compressed again.
	std::fs::remove_file(out.join("app.js.gz")).expect("remove gz");
	compress::precompress(&out, &mut cache).expect("precompress");
	assert_eq!(
		ungzip(&std::fs::read(out.join("app.js.gz")).expect("gz")),
		compressible_text().as_bytes()
	);
	let changed = format!("{} and more", compressible_text());
	std::fs::write(&page, &changed).expect("change");
	compress::precompress(&out, &mut cache).expect("precompress");
	assert_eq!(
		unbrotli(&std::fs::read(&brotli).expect("br")),
		changed.as_bytes()
	);
}

#[test]
fn precompress_keeps_only_variants_smaller_than_their_file() {
	let scratch = Scratch::new("tiny");
	std::fs::write(scratch.path.join("a.js"), "1").expect("write");
	// A variant from an earlier build of a file that no longer compresses.
	std::fs::write(scratch.path.join("a.js.br"), "stale").expect("write");
	let sizes = compress::precompress(&scratch.path, &mut PrecompressCache::default())
		.expect("precompress");
	assert!(!scratch.path.join("a.js.br").exists());
	assert!(!scratch.path.join("a.js.gz").exists());
	assert_eq!(
		sizes[&scratch.path.join("a.js")],
		Sizes {
			raw: 1,
			gzip: 1,
			brotli: 1,
		},
		"with no variant, a server sends the file itself"
	);
	assert_eq!(
		compress::measure(&scratch.path.join("a.js")).expect("measure"),
		sizes[&scratch.path.join("a.js")]
	);
}

#[test]
fn optimize_needs_a_release_build() {
	let scratch = Scratch::new("optimize-debug");
	let output = leptoskit(&scratch.path, &["build", "--optimize", "smallest"]);
	assert!(!output.status.success());
	let message = String::from_utf8_lossy(&output.stderr);
	assert!(message.contains("--release"), "{message}");
}

#[test]
fn stale_variants_go_and_unrelated_archives_stay() {
	let scratch = Scratch::new("stale");
	let dir = &scratch.path;
	std::fs::write(dir.join("app.js"), compressible_text()).expect("write");
	std::fs::write(dir.join("app.js.br"), "old").expect("write");
	std::fs::write(dir.join("gone.wasm.gz"), "orphan").expect("write");
	std::fs::write(dir.join("data.tar.gz"), "a download").expect("write");

	compress::precompress(dir, &mut PrecompressCache::default()).expect("precompress");
	assert_ne!(
		std::fs::read(dir.join("app.js.br")).expect("rewritten"),
		b"old"
	);
	assert!(
		!dir.join("gone.wasm.gz").exists(),
		"a variant of a deleted file goes"
	);
	assert!(dir.join("data.tar.gz").exists(), "archives are not ours");

	compress::remove_precompressed(dir).expect("remove");
	assert!(!dir.join("app.js.br").exists() && !dir.join("app.js.gz").exists());
	assert!(dir.join("app.js").exists());
	assert!(dir.join("data.tar.gz").exists());
}

#[test]
fn the_report_totals_the_initial_load_and_enforces_the_budget() {
	let scratch = Scratch::new("report");
	let pkg = scratch.path.join("pkg");
	std::fs::create_dir_all(pkg.join("snippets")).expect("pkg");
	std::fs::write(pkg.join("app.js"), compressible_text()).expect("write");
	std::fs::write(pkg.join("app_bg.wasm"), compressible_text()).expect("write");
	std::fs::write(pkg.join("snippets/inline.js"), compressible_text()).expect("write");
	std::fs::write(pkg.join("chunk_0.wasm"), compressible_text()).expect("write");
	std::fs::write(pkg.join("manifest.json"), "{}").expect("write");

	let report = BundleReport::measure(
		&scratch.path,
		&pkg,
		|path| path.file_name().is_some_and(|name| name == "chunk_0.wasm"),
		&BTreeMap::new(),
	)
	.expect("measure");
	let paths: Vec<(&str, bool)> = report
		.entries
		.iter()
		.map(|entry| (entry.path.as_str(), entry.lazy))
		.collect();
	assert_eq!(
		paths,
		[
			("pkg/app.js", false),
			("pkg/app_bg.wasm", false),
			("pkg/snippets/inline.js", false),
			("pkg/chunk_0.wasm", true),
		]
	);
	let one = report.entries[0].sizes;
	let initial = report.initial();
	assert_eq!(initial.raw, one.raw * 3, "the lazy chunk is not initial");
	assert!(initial.brotli < initial.raw);

	report.check_budget(None).expect("no budget");
	report.check_budget(Some(1024)).expect("well within");
	let error = report.check_budget(Some(0)).unwrap_err().to_string();
	assert!(error.contains("over bundle.budget_kib = 0"), "{error}");

	let table = report.render(Some(1024));
	assert!(table.contains("pkg/chunk_0.wasm (lazy)"), "{table}");
	assert!(table.contains("initial load"), "{table}");
	assert!(table.contains("within the 1024 KiB budget"), "{table}");
	assert!(report.render(Some(0)).contains("OVER the 0 KiB budget"));
}

#[test]
fn the_report_reuses_sizes_the_build_already_measured() {
	let scratch = Scratch::new("known");
	let pkg = scratch.path.join("pkg");
	std::fs::create_dir_all(&pkg).expect("pkg");
	std::fs::write(pkg.join("app.js"), compressible_text()).expect("write");
	let known = Sizes {
		raw: 7,
		gzip: 5,
		brotli: 3,
	};
	let report = BundleReport::measure(
		&scratch.path,
		&pkg,
		|_| false,
		&BTreeMap::from([(pkg.join("app.js"), known)]),
	)
	.expect("measure");
	assert_eq!(report.entries[0].sizes, known);
}

#[test]
fn smallest_compiles_release_clients_with_leptoskits_profile() {
	let options = CargoOptions {
		release: true,
		..CargoOptions::default()
	}
	.with_profile(wasm::SMALLEST_PROFILE, wasm::smallest_profile_config());
	let args = cargo_args(&options);
	let profile = args
		.iter()
		.position(|arg| arg == "--profile")
		.expect("--profile");
	assert_eq!(args[profile + 1], "leptoskit-smallest");
	assert!(!args.contains(&"--release".to_owned()));
	for setting in [
		"profile.leptoskit-smallest.inherits = \"release\"",
		"profile.leptoskit-smallest.opt-level = \"z\"",
		"profile.leptoskit-smallest.lto = true",
		"profile.leptoskit-smallest.codegen-units = 1",
	] {
		assert!(args.contains(&setting.to_owned()), "{setting} in {args:?}");
	}

	let debug = CargoOptions::default().with_profile(wasm::SMALLEST_PROFILE, Vec::new());
	assert!(
		!cargo_args(&debug).contains(&"--profile".to_owned()),
		"dev builds ignore the profile"
	);
	let user = CargoOptions {
		release: true,
		..CargoOptions::default()
	}
	.with_profile("wasm-release", Vec::new());
	let args = cargo_args(&user);
	let profile = args
		.iter()
		.position(|arg| arg == "--profile")
		.expect("--profile");
	assert_eq!(args[profile + 1], "wasm-release");
}

fn cargo_args(options: &CargoOptions) -> Vec<String> {
	wasm::cargo_build_command(Path::new("."), "app", options)
		.get_args()
		.map(|arg| arg.to_string_lossy().into_owned())
		.collect()
}

#[test]
fn the_compiled_client_is_the_cdylibs_wasm_in_cargos_messages() {
	let messages = r#"{"reason":"compiler-artifact","target":{"kind":["lib"]},"filenames":["/t/libserde.rlib"]}
not json
{"reason":"compiler-artifact","target":{"kind":["cdylib","rlib"]},"filenames":["/t/wasm32-unknown-unknown/bench/app.wasm","/t/wasm32-unknown-unknown/bench/libapp.rlib"]}
{"reason":"build-finished","success":true}
"#;
	assert_eq!(
		wasm::wasm_in_messages(std::io::Cursor::new(messages)).expect("parse"),
		Some(PathBuf::from("/t/wasm32-unknown-unknown/bench/app.wasm")),
		"the profile's directory comes from cargo, whatever its name"
	);
	assert_eq!(
		wasm::wasm_in_messages(std::io::Cursor::new("{\"reason\":\"build-finished\"}\n"))
			.expect("parse"),
		None
	);
}

/// Runs the built CLI in `dir`. The fixture builds it starts must not
/// inherit a coverage run's `-C instrument-coverage`, which cannot build for
/// wasm; the CLI itself still records coverage through `LLVM_PROFILE_FILE`.
fn leptoskit(dir: &Path, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args(args)
		.current_dir(dir)
		.env_remove("RUSTFLAGS")
		.output()
		.expect("run leptoskit")
}

/// A client-rendered app in one crate: a cdylib client, a server binary, and
/// a public page. Binaryen is not required: `wasm_opt` is off.
fn slim_app(dir: &Path) {
	std::fs::write(
		dir.join(leptoskit_config::CONFIG_FILE),
		"[app]\nname = \"slim\"\n\n[bundle]\nwasm_opt = false\nbudget_kib = 1\n",
	)
	.expect("config");
	std::fs::write(
		dir.join("Cargo.toml"),
		r#"[package]
name = "slim"
version = "0.1.0"
edition = "2024"

[workspace]

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
wasm-bindgen = "=0.2.100"
"#,
	)
	.expect("manifest");
	std::fs::create_dir_all(dir.join("src")).expect("src");
	std::fs::write(
		dir.join("src/lib.rs"),
		"#[wasm_bindgen::prelude::wasm_bindgen]\npub fn greet(name: &str) -> String \
		 {\n\tformat!(\"hello, {name}\")\n}\n",
	)
	.expect("lib");
	std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").expect("main");
	std::fs::create_dir_all(dir.join("public")).expect("public");
	std::fs::write(dir.join("public/index.html"), compressible_text()).expect("page");
}

/// A release build over a 1 KiB budget: the report prints,
/// the build fails on the budget after writing the precompressed output, a
/// `--optimize smallest` build compiles with leptoskit's profile, and a dev
/// build afterwards removes the stale variants.
///
/// Ignored by default because it compiles for `wasm32-unknown-unknown` and
/// fetches dependencies; the `wasm end-to-end` CI job runs it.
#[test]
#[ignore = "builds for wasm32 and fetches dependencies; run by the `wasm end-to-end` CI job"]
fn release_builds_precompress_report_and_hold_the_budget() {
	let scratch = Scratch::new("budget");
	let project = &scratch.path;
	slim_app(project);

	let release = leptoskit(project, &["build", "--release"]);
	let out = String::from_utf8_lossy(&release.stdout);
	let err = String::from_utf8_lossy(&release.stderr);
	assert!(
		!release.status.success(),
		"a 1 KiB budget must fail:\n{out}"
	);
	assert!(err.contains("over bundle.budget_kib = 1"), "{err}");
	assert!(out.contains("client bundle (optimize = \"fast\")"), "{out}");
	assert!(out.contains("pkg/slim_bg.wasm"), "{out}");
	assert!(out.contains("OVER the 1 KiB budget"), "{out}");
	let wasm = project.join("dist/pkg/slim_bg.wasm");
	let brotli = std::fs::read(project.join("dist/pkg/slim_bg.wasm.br")).expect("br");
	assert_eq!(unbrotli(&brotli), std::fs::read(&wasm).expect("wasm"));
	assert!(project.join("dist/index.html.gz").is_file());

	let smallest = leptoskit(project, &["build", "--release", "--optimize", "smallest"]);
	let out = String::from_utf8_lossy(&smallest.stdout);
	assert!(
		out.contains("client bundle (optimize = \"smallest\")"),
		"{out}"
	);
	let target = wasm::cargo_metadata(project)
		.expect("metadata")
		.target_directory
		.join("wasm32-unknown-unknown");
	assert!(
		target.join("leptoskit-smallest/slim.wasm").is_file(),
		"smallest builds compile with leptoskit's profile"
	);

	// `cargo_profile` replaces optimize's profile, and the report says so.
	let config = project.join(leptoskit_config::CONFIG_FILE);
	let original = std::fs::read_to_string(&config).expect("config");
	std::fs::write(&config, format!("{original}cargo_profile = \"release\"\n")).expect("config");
	let own = leptoskit(project, &["build", "--release"]);
	let out = String::from_utf8_lossy(&own.stdout);
	assert!(
		out.contains("client bundle (cargo_profile = \"release\")"),
		"{out}"
	);
	std::fs::write(&config, original).expect("config");

	let dev = leptoskit(project, &["build"]);
	assert!(
		dev.status.success(),
		"{}",
		String::from_utf8_lossy(&dev.stderr)
	);
	assert!(
		!project.join("dist/pkg/slim_bg.wasm.br").exists(),
		"a dev build must not leave a release build's variants to be served"
	);
}
