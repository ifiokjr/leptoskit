//! Real application generation journey.
use super::*;

/// The whole flow on a real leptos app: build the server, export its spec by
/// running it, generate every client, and compile the Rust and CLI crates.
///
/// Ignored by default because it fetches leptos from the registry and builds
/// it several times. Run it with
/// `cargo nextest run --package leptoskit_cli --run-ignored only --
/// fns_end_to_end`.
#[test]
#[ignore = "needs a registry fetch and a full leptos build"]
fn fns_end_to_end_exports_generates_and_checks() {
	let scratch = Scratch::new("e2e");
	let root = &scratch.path;
	setup(root);

	let listed = leptoskit(root, &["rpc", "list"]);
	assert!(listed.status.success(), "{}", stderr(&listed));
	assert!(
		stdout(&listed).contains("POST /api/list_todos"),
		"{}",
		stdout(&listed)
	);
	assert!(!stdout(&listed).contains("flush"));

	for (target, dir) in [
		("rust", "clients/rust/src/client.rs"),
		("cli", "clients/cli/src/main.rs"),
		("typescript", "clients/typescript/index.ts"),
		("python", "clients/python"),
		("go", "clients/go"),
		("dart", "clients/dart"),
		("openapi", "clients/openapi/openapi.json"),
	] {
		let generated = leptoskit(root, &["rpc", "generate", "--target", target]);
		assert!(
			generated.status.success(),
			"{target}: {}\n{}",
			stdout(&generated),
			stderr(&generated)
		);
		assert!(root.join(dir).exists(), "{target}: {dir} missing");
	}

	let checked = leptoskit(root, &["rpc", "generate", "--target", "rust", "--build"]);
	assert!(
		checked.status.success(),
		"{}\n{}",
		stdout(&checked),
		stderr(&checked)
	);
	assert!(
		stdout(&checked).contains("rust: checked"),
		"{}",
		stdout(&checked)
	);
	let client = read(&root.join("clients/rust/src/client.rs"));
	assert!(client.contains("pub async fn list_todos("));
	assert!(
		client.contains("/// * `limit` — How many to return."),
		"argument docs reach the Rust client"
	);
	assert!(!client.contains("flush"));
}

fn setup(root: &Path) {
	let fns_crate = Path::new(env!("CARGO_MANIFEST_DIR")).join("../leptoskit_rpc");
	let fns_crate = std::fs::canonicalize(fns_crate).expect("leptoskit_rpc path");
	std::fs::write(
		root.join("Cargo.toml"),
		format!(
			r#"[package]
name = "todos-app"
version = "0.3.0"
edition = "2021"

[lib]

[[bin]]
name = "todos-app"
path = "src/main.rs"
required-features = ["ssr"]

[dependencies]
leptos = "0.9.0-rc0"
leptoskit_rpc = {{ path = "{fns}" }}
schemars = "1"
serde = {{ version = "1", features = ["derive"] }}

[features]
ssr = ["leptos/ssr", "leptoskit_rpc/ssr"]

[workspace]
"#,
			fns = fns_crate.display()
		),
	)
	.expect("manifest");
	std::fs::create_dir_all(root.join("src")).expect("src");
	std::fs::write(
		root.join("src/lib.rs"),
		r#"pub mod api {
	use leptos::prelude::*;
	use leptoskit_rpc::api;
	use schemars::JsonSchema;
	use serde::{Deserialize, Serialize};

	#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
	pub struct Todo { pub id: u64, pub title: String }

	/// Lists todos.
	///
	/// # Arguments
	///
	/// * `limit` - How many to return.
	#[api]
	#[server]
	pub async fn list_todos(limit: Option<u32>) -> Result<Vec<Todo>, ServerFnError> {
		let _ = limit;
		Ok(Vec::new())
	}

	/// Operators only.
	#[api(internal)]
	#[server]
	pub async fn flush() -> Result<(), ServerFnError> { Ok(()) }
}

pub const NAME: &str = "todos";
"#,
	)
	.expect("lib");
	std::fs::write(
		root.join("src/main.rs"),
		"fn main() {\n\tleptoskit_rpc::spec::export_if_requested();\n\tprintln!(\"{}\", \
		 todos_app::NAME);\n}\n",
	)
	.expect("main");
	std::fs::write(
		root.join("leptoskit.toml"),
		"[app]\nname = \"todos-app\"\nversion = \"0.3.0\"\ndefault_platform = \
		 \"server\"\n\n[build]\nfeatures = [\"ssr\"]\n",
	)
	.expect("config");
}
