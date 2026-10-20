//! Render mode tests: the features each side of a build gets, finding the
//! server binary, and running it with the leptos environment.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_cli::render;
use leptoskit_cli::server;
use leptoskit_cli::server::LeptosEnv;
use leptoskit_cli::server::SiteEnv;
use leptoskit_cli::wasm::MetadataDependency;
use leptoskit_cli::wasm::MetadataPackage;
use leptoskit_config::RenderMode;

struct Scratch {
	path: PathBuf,
}

impl Scratch {
	fn new(tag: &str) -> Self {
		static COUNTER: AtomicU64 = AtomicU64::new(0);
		let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
		let path = std::env::temp_dir().join(format!(
			"leptoskit_render_test_{tag}_{}_{}",
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

fn leptoskit(dir: &Path, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args(args)
		.current_dir(dir)
		.output()
		.expect("run leptoskit")
}

fn text(bytes: &[u8]) -> String {
	String::from_utf8_lossy(bytes).into_owned()
}

fn package(
	name: &str,
	features: &[&str],
	dependencies: &[(&str, Option<&str>)],
) -> MetadataPackage {
	MetadataPackage {
		name: name.to_owned(),
		manifest_path: PathBuf::from(format!("/{name}/Cargo.toml")),
		targets: Vec::new(),
		features: features
			.iter()
			.map(|feature| ((*feature).to_owned(), Vec::new()))
			.collect::<BTreeMap<_, _>>(),
		dependencies: dependencies
			.iter()
			.map(|(name, rename)| {
				MetadataDependency {
					name: (*name).to_owned(),
					rename: rename.map(str::to_owned),
				}
			})
			.collect(),
	}
}

#[test]
fn csr_adds_no_features() {
	let server = package("app", &[], &[]);
	let client = package("app_client", &[], &[]);
	let features = render::mode_features(RenderMode::Csr, &server, Some(&client)).expect("csr");
	assert_eq!(features, render::ModeFeatures::default());
}

#[test]
fn ssr_builds_the_server_with_ssr_and_the_client_with_hydrate() {
	let server = package("app", &["ssr"], &[("leptos", None)]);
	let client = package("app_client", &["hydrate"], &[("leptos", None)]);
	let features = render::mode_features(RenderMode::Ssr, &server, Some(&client)).expect("ssr");
	assert_eq!(features.server, ["ssr"]);
	assert_eq!(features.client, ["hydrate"]);
}

#[test]
fn islands_turn_on_leptos_islands_under_its_local_name() {
	let server = package("app", &["ssr"], &[("leptos", Some("lp"))]);
	let client = package("app_client", &["hydrate"], &[("leptos", None)]);
	let features =
		render::mode_features(RenderMode::Islands, &server, Some(&client)).expect("islands");
	assert_eq!(features.server, ["ssr", "lp/islands"]);
	assert_eq!(features.client, ["hydrate", "leptos/islands"]);
}

#[test]
fn a_server_without_a_client_only_gets_server_features() {
	let server = package("app", &["ssr"], &[("leptos", None)]);
	let features = render::mode_features(RenderMode::Islands, &server, None).expect("islands");
	assert_eq!(features.server, ["ssr", "leptos/islands"]);
	assert!(features.client.is_empty());
}

#[test]
fn missing_mode_features_name_the_package_and_the_fix() {
	let server = package("app", &[], &[]);
	let error = render::mode_features(RenderMode::Ssr, &server, None)
		.unwrap_err()
		.to_string();
	assert!(error.contains("`app`"), "{error}");
	assert!(error.contains("`ssr` feature"), "{error}");
	assert!(error.contains("app.render = \"csr\""), "{error}");

	let server = package("app", &["ssr"], &[]);
	let client = package("app_client", &[], &[]);
	let error = render::mode_features(RenderMode::Ssr, &server, Some(&client))
		.unwrap_err()
		.to_string();
	assert!(error.contains("`app_client`"), "{error}");
	assert!(error.contains("`hydrate` feature"), "{error}");
}

#[test]
fn islands_need_a_direct_leptos_dependency() {
	let server = package("app", &["ssr"], &[]);
	let error = render::mode_features(RenderMode::Islands, &server, None)
		.unwrap_err()
		.to_string();
	assert!(error.contains("does not depend on leptos"), "{error}");
}

#[test]
fn server_binaries_come_from_cargo_artifact_messages() {
	let messages = r#"{"reason":"compiler-artifact","target":{"name":"serde","kind":["lib"]},"executable":null}
{"reason":"build-script-executed","package_id":"x"}
not json at all
{"reason":"compiler-artifact","target":{"name":"app","kind":["cdylib","rlib"]},"executable":null}
{"reason":"compiler-artifact","target":{"name":"app","kind":["bin"]},"executable":"/target/debug/app"}
{"reason":"build-finished","success":true}
"#;
	let binaries = server::binaries_in_messages(Cursor::new(messages)).expect("parse");
	assert_eq!(
		binaries,
		[("app".to_owned(), PathBuf::from("/target/debug/app"))]
	);
}

#[test]
fn the_server_is_the_binary_named_after_the_package_or_the_only_one() {
	let bins = |names: &[&str]| -> Vec<(String, PathBuf)> {
		names
			.iter()
			.map(|name| ((*name).to_owned(), PathBuf::from(format!("/bin/{name}"))))
			.collect()
	};
	assert_eq!(
		server::pick_server("app", bins(&["tool", "app"])),
		Some(PathBuf::from("/bin/app"))
	);
	assert_eq!(
		server::pick_server("app", bins(&["web"])),
		Some(PathBuf::from("/bin/web"))
	);
	assert_eq!(server::pick_server("app", bins(&["a", "b"])), None);
	assert_eq!(server::pick_server("app", bins(&[])), None);
}

#[test]
fn site_env_points_leptos_at_the_built_site() {
	let site = SiteEnv {
		output_name: "app".to_owned(),
		site_root: PathBuf::from("/project/dist"),
		address: "127.0.0.1:3000".parse::<SocketAddr>().expect("address"),
		env: LeptosEnv::Prod,
	};
	let vars: BTreeMap<&str, String> = site.vars().into_iter().collect();
	assert_eq!(vars["LEPTOS_OUTPUT_NAME"], "app");
	assert_eq!(vars["LEPTOS_SITE_ROOT"], "/project/dist");
	assert_eq!(vars["LEPTOS_SITE_PKG_DIR"], "pkg");
	assert_eq!(vars["LEPTOS_SITE_ADDR"], "127.0.0.1:3000");
	assert_eq!(vars["LEPTOS_ENV"], "PROD");
}

/// One package that is both the server binary and the cdylib client, the
/// layout most leptos apps use. It only compiles when each side gets its
/// own mode feature and not the other's, and its server prints the leptos
/// environment it was started with.
fn single_crate_app(dir: &Path, render: &str) {
	std::fs::write(
		dir.join(leptoskit_config::CONFIG_FILE),
		format!("[app]\nname = \"solo\"\nrender = \"{render}\"\n\n[bundle]\nwasm_opt = false\n"),
	)
	.expect("config");
	std::fs::write(
		dir.join("Cargo.toml"),
		r#"[package]
name = "solo"
version = "0.1.0"
edition = "2024"

[workspace]

[lib]
crate-type = ["cdylib", "rlib"]

[features]
ssr = []
hydrate = []

[dependencies]
wasm-bindgen = "=0.2.100"
"#,
	)
	.expect("manifest");
	std::fs::create_dir_all(dir.join("src")).expect("src");
	std::fs::write(
		dir.join("src/lib.rs"),
		r#"#[cfg(all(target_arch = "wasm32", not(feature = "hydrate")))]
compile_error!("the client was built without `hydrate`");
#[cfg(all(target_arch = "wasm32", feature = "ssr"))]
compile_error!("`ssr` leaked into the client");

#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {}
"#,
	)
	.expect("lib");
	std::fs::write(
		dir.join("src/main.rs"),
		r#"#[cfg(not(feature = "ssr"))]
compile_error!("the server was built without `ssr`");
#[cfg(feature = "hydrate")]
compile_error!("`hydrate` leaked into the server");

fn main() {
	for key in ["LEPTOS_OUTPUT_NAME", "LEPTOS_SITE_ROOT", "LEPTOS_SITE_PKG_DIR", "LEPTOS_ENV"] {
		println!("{key}={}", std::env::var(key).unwrap_or_default());
	}
}
"#,
	)
	.expect("main");
}

/// Run with `cargo nextest run --package leptoskit_cli --run-ignored only`.
#[test]
#[ignore = "needs the wasm32 target and a registry fetch"]
fn ssr_builds_each_side_with_its_feature_and_serve_runs_the_server() {
	let scratch = Scratch::new("ssr");
	let root = &scratch.path;
	single_crate_app(root, "ssr");

	let build = leptoskit(root, &["build"]);
	assert!(
		build.status.success(),
		"build failed:\n{}\n{}",
		text(&build.stdout),
		text(&build.stderr)
	);
	assert!(text(&build.stdout).contains("(ssr render)"));
	assert!(root.join("dist/pkg/solo.js").is_file());
	assert!(root.join("dist/pkg/solo_bg.wasm").is_file());

	// The stand-in server prints its environment and exits, so `serve`
	// returns once it has run it.
	let serve = leptoskit(root, &["serve", "--port", "4891"]);
	let out = text(&serve.stdout);
	assert!(
		serve.status.success(),
		"serve failed:\n{out}\n{}",
		text(&serve.stderr)
	);
	assert!(out.contains("LEPTOS_OUTPUT_NAME=solo\n"), "{out}");
	assert!(
		out.contains(&format!(
			"LEPTOS_SITE_ROOT={}\n",
			root.join("dist").display()
		)),
		"{out}"
	);
	assert!(out.contains("LEPTOS_SITE_PKG_DIR=pkg\n"), "{out}");
	assert!(out.contains("LEPTOS_ENV=PROD\n"), "{out}");
}

#[test]
fn csr_leaves_both_sides_without_mode_features() {
	let scratch = Scratch::new("csr");
	let root = &scratch.path;
	single_crate_app(root, "csr");
	let build = leptoskit(root, &["build", "--platform", "server"]);
	assert!(!build.status.success(), "csr must not pass `ssr`");
	assert!(
		text(&build.stderr).contains("the server was built without `ssr`"),
		"{}",
		text(&build.stderr)
	);
}

#[test]
fn ssr_fails_clearly_when_the_server_has_no_ssr_feature() {
	let scratch = Scratch::new("missing");
	let root = &scratch.path;
	std::fs::write(
		root.join(leptoskit_config::CONFIG_FILE),
		"[app]\nname = \"plain\"\nrender = \"ssr\"\ndefault_platform = \"server\"\n",
	)
	.expect("config");
	std::fs::write(
		root.join("Cargo.toml"),
		"[package]\nname = \"plain\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n",
	)
	.expect("manifest");
	std::fs::create_dir_all(root.join("src")).expect("src");
	std::fs::write(root.join("src/main.rs"), "fn main() {}\n").expect("main");

	let build = leptoskit(root, &["build"]);
	assert!(!build.status.success());
	let message = text(&build.stderr);
	assert!(message.contains("declares no such feature"), "{message}");
}
