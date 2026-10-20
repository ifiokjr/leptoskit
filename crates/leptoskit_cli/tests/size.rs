//! Bundle size tests: the size profile, precompression, and the report and
//! budget release builds print.

use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_cli::compress;
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

	compress::precompress(&scratch.path).expect("precompress");

	let brotli = std::fs::read(pkg.join("app.js.br")).expect("brotli variant");
	let gzip = std::fs::read(pkg.join("app.js.gz")).expect("gzip variant");
	assert_eq!(unbrotli(&brotli), text.as_bytes());
	assert_eq!(ungzip(&gzip), text.as_bytes());
	assert!(brotli.len() < text.len() && gzip.len() < text.len());
	assert!(
		!scratch.path.join("photo.png.br").exists(),
		"images are already compressed"
	);
}

#[test]
fn precompress_keeps_only_variants_smaller_than_their_file() {
	let scratch = Scratch::new("tiny");
	std::fs::write(scratch.path.join("a.js"), "1").expect("write");
	// A variant from an earlier build of a file that no longer compresses.
	std::fs::write(scratch.path.join("a.js.br"), "stale").expect("write");
	compress::precompress(&scratch.path).expect("precompress");
	assert!(!scratch.path.join("a.js.br").exists());
	assert!(!scratch.path.join("a.js.gz").exists());
}

#[test]
fn stale_variants_go_and_unrelated_archives_stay() {
	let scratch = Scratch::new("stale");
	let dir = &scratch.path;
	std::fs::write(dir.join("app.js"), compressible_text()).expect("write");
	std::fs::write(dir.join("app.js.br"), "old").expect("write");
	std::fs::write(dir.join("gone.wasm.gz"), "orphan").expect("write");
	std::fs::write(dir.join("data.tar.gz"), "a download").expect("write");

	compress::precompress(dir).expect("precompress");
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

	let report = BundleReport::measure(&scratch.path, &pkg, |path| {
		path.file_name().is_some_and(|name| name == "chunk_0.wasm")
	})
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
	assert!(error.contains("over build.budget_kib = 0"), "{error}");

	let table = report.render(Some(1024));
	assert!(table.contains("pkg/chunk_0.wasm (lazy)"), "{table}");
	assert!(table.contains("initial load"), "{table}");
	assert!(table.contains("within the 1024 KiB budget"), "{table}");
	assert!(report.render(Some(0)).contains("OVER the 0 KiB budget"));
}

#[test]
fn release_clients_compile_with_the_size_profile() {
	let options = CargoOptions {
		release: true,
		..CargoOptions::default()
	}
	.with_profile(wasm::SIZE_PROFILE, wasm::size_profile_config());
	assert_eq!(options.profile_dir(), "leptoskit-wasm");
	let command = wasm::cargo_build_command(Path::new("."), "app", &options);
	let args: Vec<String> = command
		.get_args()
		.map(|arg| arg.to_string_lossy().into_owned())
		.collect();
	let profile = args
		.iter()
		.position(|arg| arg == "--profile")
		.expect("--profile");
	assert_eq!(args[profile + 1], "leptoskit-wasm");
	assert!(!args.contains(&"--release".to_owned()));
	for setting in [
		"profile.leptoskit-wasm.inherits = \"release\"",
		"profile.leptoskit-wasm.opt-level = \"z\"",
		"profile.leptoskit-wasm.lto = true",
		"profile.leptoskit-wasm.codegen-units = 1",
	] {
		assert!(args.contains(&setting.to_owned()), "{setting} in {args:?}");
	}

	let debug = CargoOptions::default().with_profile(wasm::SIZE_PROFILE, Vec::new());
	assert_eq!(
		debug.profile_dir(),
		"debug",
		"dev builds ignore the profile"
	);
	let user = CargoOptions {
		release: true,
		..CargoOptions::default()
	}
	.with_profile("wasm-release", Vec::new());
	assert_eq!(user.profile_dir(), "wasm-release");
}

fn leptoskit(dir: &Path, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args(args)
		.current_dir(dir)
		.output()
		.expect("run leptoskit")
}

/// A release build of the scaffold over a 1 KiB budget: the report prints,
/// the build fails on the budget after writing the precompressed output, and
/// a dev build afterwards removes the stale variants.
///
/// Run with `cargo nextest run --package leptoskit_cli --run-ignored only`.
#[test]
#[ignore = "needs the wasm32 target and a registry fetch"]
fn release_builds_precompress_report_and_hold_the_budget() {
	let scratch = Scratch::new("budget");
	let created = leptoskit(&scratch.path, &["new", "slim"]);
	assert!(
		created.status.success(),
		"{}",
		String::from_utf8_lossy(&created.stderr)
	);
	let project = scratch.path.join("slim");
	let config_path = project.join(leptoskit_config::CONFIG_FILE);
	let mut config: leptoskit_config::LeptoskitConfig =
		toml::from_str(&std::fs::read_to_string(&config_path).expect("config")).expect("parse");
	// Keep the test independent of binaryen being installed.
	config.build.wasm_opt = false;
	config.build.budget_kib = Some(1);
	std::fs::write(&config_path, config.to_toml_string().expect("serialize")).expect("write");

	let release = leptoskit(&project, &["build", "--release"]);
	let out = String::from_utf8_lossy(&release.stdout);
	let err = String::from_utf8_lossy(&release.stderr);
	assert!(
		!release.status.success(),
		"a 1 KiB budget must fail:\n{out}"
	);
	assert!(err.contains("over build.budget_kib = 1"), "{err}");
	assert!(out.contains("pkg/slim_client_bg.wasm"), "{out}");
	assert!(out.contains("OVER the 1 KiB budget"), "{out}");
	let wasm = project.join("dist/pkg/slim_client_bg.wasm");
	let brotli = std::fs::read(project.join("dist/pkg/slim_client_bg.wasm.br")).expect("br");
	assert_eq!(unbrotli(&brotli), std::fs::read(&wasm).expect("wasm"));
	assert!(project.join("dist/index.html.gz").is_file());

	let dev = leptoskit(&project, &["build"]);
	assert!(
		dev.status.success(),
		"{}",
		String::from_utf8_lossy(&dev.stderr)
	);
	assert!(
		!project.join("dist/pkg/slim_client_bg.wasm.br").exists(),
		"a dev build must not leave a release build's variants to be served"
	);
}
