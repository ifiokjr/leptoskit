//! `leptoskit rpc` tests: listing, spec formats, and generation from a spec
//! file, driven through the built `leptoskit` binary in temporary projects.
//! The spec fixture was exported from a real leptos app with four `#[api]`
//! functions, one of them internal.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

const SPEC: &str = include_str!("fixtures/fns_spec.json");

#[path = "rpc/concurrent.rs"]
mod concurrent;

struct Scratch {
	path: PathBuf,
}

impl Scratch {
	fn new(tag: &str) -> Self {
		static COUNTER: AtomicU64 = AtomicU64::new(0);
		let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
		let path = std::env::temp_dir().join(format!(
			"leptoskit_rpc_test_{tag}_{}_{}",
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

fn stdout(output: &Output) -> String {
	String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
	String::from_utf8_lossy(&output.stderr).into_owned()
}

fn read(path: &Path) -> String {
	std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// A minimal project with the spec fixture next to it.
fn project(dir: &Path, fns_config: &str) {
	std::fs::write(
		dir.join("leptoskit.toml"),
		format!(
			"[app]\nname = \"todos-app\"\nversion = \"0.3.0\"\ndefault_platform = \
			 \"server\"\n\n{fns_config}"
		),
	)
	.expect("config");
	std::fs::write(dir.join("spec.json"), SPEC).expect("spec");
}

#[test]
fn list_hides_internal_functions_unless_asked() {
	let scratch = Scratch::new("list");
	project(&scratch.path, "");

	let output = leptoskit(&scratch.path, &["rpc", "list", "--from", "spec.json"]);
	assert!(output.status.success(), "{}", stderr(&output));
	let listing = stdout(&output);
	assert!(listing.contains("list_todos"), "{listing}");
	assert!(listing.contains("POST /api/list_todos"), "{listing}");
	assert!(listing.contains("GET /api/get_todo"), "{listing}");
	assert!(
		!listing.contains("flush_caches"),
		"internal stays hidden: {listing}"
	);
	// The # Arguments docs are the first-line summaries.
	assert!(listing.contains("Lists todos."), "{listing}");

	let output = leptoskit(
		&scratch.path,
		&["rpc", "list", "--from", "spec.json", "--include-internal"],
	);
	assert!(output.status.success(), "{}", stderr(&output));
	assert!(stdout(&output).contains("flush_caches"));
	assert!(stdout(&output).contains("(internal)"));
}

#[test]
fn spec_writes_every_format() {
	let scratch = Scratch::new("spec");
	project(&scratch.path, "[rpc]\ntitle = \"Todos\"\n");

	let json = leptoskit(&scratch.path, &["rpc", "spec", "--from", "spec.json"]);
	assert!(json.status.success(), "{}", stderr(&json));
	let spec: serde_json::Value = serde_json::from_str(&stdout(&json)).expect("spec json");
	assert_eq!(spec["leptoskit_rpc_spec"], 1);
	let names: Vec<&str> = spec["functions"]
		.as_array()
		.expect("functions")
		.iter()
		.filter_map(|function| function["name"].as_str())
		.collect();
	assert_eq!(names, ["create_todo", "get_todo", "list_todos"]);

	let openapi = leptoskit(
		&scratch.path,
		&["rpc", "spec", "--from", "spec.json", "--format", "openapi"],
	);
	assert!(openapi.status.success(), "{}", stderr(&openapi));
	let document: serde_json::Value =
		serde_json::from_str(&stdout(&openapi)).expect("openapi json");
	assert_eq!(document["openapi"], "3.1.0");
	assert_eq!(document["info"]["title"], "Todos");
	assert!(document["paths"]["/api/list_todos"]["post"].is_object());
	assert!(
		document["paths"]["/api/flush_caches"].is_null(),
		"internal functions leave the public document"
	);

	assert!(document["components"]["schemas"]["Todo"].is_object());

	let yaml = leptoskit(
		&scratch.path,
		&[
			"rpc",
			"spec",
			"--from",
			"spec.json",
			"--format",
			"openapi-yaml",
			"--out",
			"docs/openapi.yaml",
		],
	);
	assert!(yaml.status.success(), "{}", stderr(&yaml));
	let written = read(&scratch.path.join("docs/openapi.yaml"));
	assert!(written.contains("\"openapi\": \"3.1.0\""));
	assert!(stdout(&yaml).contains("wrote 3 function(s)"));
}

#[test]
fn generate_writes_every_client() {
	let scratch = Scratch::new("generate");
	let fns_config = "[rpc]\nservers = [\"https://todos.example\"]\n\n[[rpc.targets]]\nlanguage = \
	                  \"typescript\"\npackage = \"@acme/todos\"\n\n[[rpc.targets]]\nlanguage = \
	                  \"python\"\npackage = \"todos-api\"\n\n[[rpc.targets]]\nlanguage = \
	                  \"go\"\npackage = \"example.com/todos\"\n\n[[rpc.targets]]\nlanguage = \
	                  \"rust\"\n\n[[rpc.targets]]\nlanguage = \
	                  \"dart\"\n\n[[rpc.targets]]\nlanguage = \
	                  \"cli\"\n\n[[rpc.targets]]\nlanguage = \"openapi\"\nout = \"docs/openapi\"\n";
	project(&scratch.path, fns_config);
	let output = leptoskit(&scratch.path, &["rpc", "generate", "--from", "spec.json"]);
	assert!(
		output.status.success(),
		"{}\n{}",
		stdout(&output),
		stderr(&output)
	);
	let report = stdout(&output);
	for language in [
		"typescript:",
		"python:",
		"go:",
		"rust:",
		"dart:",
		"cli:",
		"openapi:",
	] {
		assert!(report.contains(language), "{report}");
	}

	assert_typescript_client(&scratch.path);

	let client = read(&scratch.path.join("clients/python/todos_api/client.py"));
	assert!(client.contains("def list_todos("), "{client}");
	assert!(
		client.contains(
			"Arguments:\n\t\t\tfilter: Which todos to list.\n\t\t\tlimit: How many to return."
		),
		"the Arguments section lands in the method docstring: {client}"
	);
	assert!(!client.contains("flush_caches"), "internal stays out");

	let go = read(&scratch.path.join("clients/go/client.go"));
	assert!(go.contains("package todos"), "{go}");
	assert!(go.contains("func (c *Client) ListTodos("), "{go}");
	assert!(
		go.contains("filter is Which todos to list."),
		"argument docs land in the Go doc comment: {go}"
	);

	let rust = read(&scratch.path.join("clients/rust/src/client.rs"));
	assert!(rust.contains("pub async fn list_todos("), "{rust}");
	assert!(
		rust.contains("/// * `filter` — Which todos to list."),
		"{rust}"
	);

	let dart = read(&scratch.path.join("clients/dart/lib/todos_app.dart"));
	assert!(dart.contains("Future<List<Todo>> listTodos("), "{dart}");
	assert!(
		dart.contains("/// - `filter`: Which todos to list."),
		"{dart}"
	);

	assert_cli_docs(&scratch.path);

	assert!(scratch.path.join("docs/openapi/openapi.json").is_file());
	assert!(scratch.path.join("docs/openapi/openapi.yaml").is_file());
	assert!(
		!scratch.path.join("clients/openapi").exists(),
		"the configured directory replaces the default"
	);
}

#[test]
fn generate_honors_exclude_globs_and_explicit_targets() {
	let scratch = Scratch::new("exclude");
	project(&scratch.path, "[rpc]\nexclude = [\"get_*\"]\n");

	let output = leptoskit(
		&scratch.path,
		&[
			"rpc",
			"generate",
			"--from",
			"spec.json",
			"--target",
			"typescript",
			"--out",
			"web/api",
		],
	);
	assert!(output.status.success(), "{}", stderr(&output));
	let index = read(&scratch.path.join("web/api/index.ts"));
	assert!(index.contains("listTodos("), "{index}");
	assert!(!index.contains("getTodo("), "excluded by glob: {index}");
	assert!(
		!scratch.path.join("clients").exists(),
		"only the requested target is written"
	);
}

#[test]
fn regeneration_keeps_scaffolding_unless_asked_to_rewrite_it() {
	let scratch = Scratch::new("scaffold");
	project(&scratch.path, "");
	let args = [
		"rpc",
		"generate",
		"--from",
		"spec.json",
		"--target",
		"typescript",
	];

	let first = leptoskit(&scratch.path, &args);
	assert!(first.status.success(), "{}", stderr(&first));
	let package = scratch.path.join("clients/typescript/package.json");
	let index = scratch.path.join("clients/typescript/index.ts");
	assert!(package.is_file() && index.is_file());
	assert!(!stdout(&first).contains("kept"), "{}", stdout(&first));

	std::fs::write(&package, "{ \"name\": \"mine\" }\n").expect("edit");
	std::fs::write(&index, "stale").expect("stale code");
	let second = leptoskit(&scratch.path, &args);
	assert!(second.status.success(), "{}", stderr(&second));
	assert_eq!(
		read(&package),
		"{ \"name\": \"mine\" }\n",
		"the manifest is the user's"
	);
	assert!(
		read(&index).contains("export class Client"),
		"the code is regenerated"
	);
	assert!(stdout(&second).contains("kept"), "{}", stdout(&second));
	assert!(
		stdout(&second).contains("--scaffold full"),
		"{}",
		stdout(&second)
	);

	let full = leptoskit(
		&scratch.path,
		&[&args[..], &["--scaffold", "full"]].concat(),
	);
	assert!(full.status.success(), "{}", stderr(&full));
	assert!(
		read(&package).contains("\"name\": \"todos-app\""),
		"{}",
		read(&package)
	);

	let files = leptoskit(
		&scratch.path,
		&[&args[..], &["--out", "web/api", "--scaffold", "files"]].concat(),
	);
	assert!(files.status.success(), "{}", stderr(&files));
	assert!(scratch.path.join("web/api/index.ts").is_file());
	assert!(
		!scratch.path.join("web/api/package.json").exists(),
		"files mode writes no manifest"
	);

	let bad = leptoskit(
		&scratch.path,
		&[&args[..], &["--scaffold", "sometimes"]].concat(),
	);
	assert!(!bad.status.success());
	assert!(
		stderr(&bad).contains("unknown scaffold mode `sometimes`"),
		"{}",
		stderr(&bad)
	);
}

#[test]
fn generate_out_needs_a_single_target() {
	let scratch = Scratch::new("out");
	project(&scratch.path, "");
	let output = leptoskit(
		&scratch.path,
		&[
			"rpc",
			"generate",
			"--from",
			"spec.json",
			"--target",
			"typescript",
			"--target",
			"openapi",
			"--out",
			"x",
		],
	);
	assert!(!output.status.success());
	assert!(
		stderr(&output).contains("--out applies to one target"),
		"{}",
		stderr(&output)
	);
}

#[test]
fn generate_rejects_unknown_targets_and_writing_into_the_app() {
	let scratch = Scratch::new("bad-target");
	project(&scratch.path, "");
	let output = leptoskit(
		&scratch.path,
		&[
			"rpc",
			"generate",
			"--from",
			"spec.json",
			"--target",
			"cobol",
		],
	);
	assert!(!output.status.success());
	assert!(
		stderr(&output).contains("unknown target `cobol`"),
		"{}",
		stderr(&output)
	);

	// Writing into the project root would clobber the app's own source.
	let output = leptoskit(
		&scratch.path,
		&[
			"rpc",
			"generate",
			"--from",
			"spec.json",
			"--target",
			"rust",
			"--out",
			".",
		],
	);
	assert!(!output.status.success());
	assert!(
		stderr(&output).contains("refusing to write"),
		"{}",
		stderr(&output)
	);
}

#[test]
fn foreign_json_is_rejected_with_a_hint() {
	let scratch = Scratch::new("foreign");
	project(&scratch.path, "");
	std::fs::write(scratch.path.join("spec.json"), "{\"openapi\": \"3.1.0\"}").expect("spec");
	let output = leptoskit(&scratch.path, &["rpc", "list", "--from", "spec.json"]);
	assert!(!output.status.success());
	assert!(
		stderr(&output).contains("not a leptoskit RPC specification"),
		"{}",
		stderr(&output)
	);
}

#[test]
fn internal_functions_generate_when_asked() {
	let scratch = Scratch::new("internal");
	project(&scratch.path, "");
	let output = leptoskit(
		&scratch.path,
		&[
			"rpc",
			"generate",
			"--from",
			"spec.json",
			"--target",
			"typescript",
			"--include-internal",
		],
	);
	assert!(output.status.success(), "{}", stderr(&output));
	let index = read(&scratch.path.join("clients/typescript/index.ts"));
	assert!(index.contains("flushCaches("), "{index}");
}

#[path = "rpc/e2e.rs"]
mod e2e;

#[test]
fn without_targets_every_client_is_generated() {
	let scratch = Scratch::new("all-targets");
	project(&scratch.path, "");
	let output = leptoskit(&scratch.path, &["rpc", "generate", "--from", "spec.json"]);
	assert!(output.status.success(), "{}", stderr(&output));
	for dir in [
		"clients/typescript/index.ts",
		"clients/python/todos_app/client.py",
		"clients/go/client.go",
		"clients/rust/src/client.rs",
		"clients/dart/lib/todos_app.dart",
		"clients/cli/src/main.rs",
		"clients/openapi/openapi.json",
	] {
		assert!(scratch.path.join(dir).is_file(), "{dir}");
	}
}

#[test]
fn list_says_when_nothing_is_published() {
	let scratch = Scratch::new("nothing");
	project(&scratch.path, "[rpc]\nexclude = [\"*\"]\n");
	let output = leptoskit(&scratch.path, &["rpc", "list", "--from", "spec.json"]);
	assert!(output.status.success(), "{}", stderr(&output));
	assert!(
		stdout(&output).contains("no published functions"),
		"{}",
		stdout(&output)
	);
}

/// A dependency-free server binary that speaks the export protocol: it writes
/// `main_body`'s behavior when `LEPTOSKIT_RPC_SPEC_OUT` is set. Each fake gets
/// its own package name: under coverage every build shares one
/// `CARGO_TARGET_DIR`, and two same-named binaries would race.
fn fake_server(dir: &Path, name: &str, main_body: &str) {
	std::fs::write(
		dir.join("leptoskit.toml"),
		format!(
			"[app]\nname = \"{name}\"\nversion = \"4.5.6\"\ndefault_platform = \
			 \"server\"\n\n[rpc]\ntitle = \"Fake RPC\"\n"
		),
	)
	.expect("config");
	std::fs::write(
		dir.join("Cargo.toml"),
		format!(
			"[package]\nname = \"{name}\"\nversion = \"4.5.6\"\nedition = \
			 \"2021\"\n\n[[bin]]\nname = \"{name}\"\npath = \"src/main.rs\"\n\n[workspace]\n"
		),
	)
	.expect("manifest");
	std::fs::create_dir_all(dir.join("src")).expect("src");
	std::fs::write(
		dir.join("src/main.rs"),
		format!("fn main() {{\n{main_body}\n}}\n"),
	)
	.expect("main");
}

#[test]
fn spec_is_exported_by_running_the_server_binary() {
	let scratch = Scratch::new("export");
	fake_server(
		&scratch.path,
		"fake-server-export",
		r##"	if let Some(out) = std::env::var_os("LEPTOSKIT_RPC_SPEC_OUT") {
		let title = std::env::var("LEPTOSKIT_RPC_SPEC_TITLE").unwrap_or_default();
		let version = std::env::var("LEPTOSKIT_RPC_SPEC_VERSION").unwrap_or_default();
		std::fs::write(
			out,
			format!(
				r#"{{"leptoskit_rpc_spec":1,"title":"{title}","version":"{version}","servers":[],"functions":[],"schemas":{{}}}}"#
			),
		)
		.unwrap();
		std::process::exit(0);
	}
	println!("serving forever");"##,
	);
	let output = leptoskit(&scratch.path, &["rpc", "spec"]);
	assert!(
		output.status.success(),
		"{}\n{}",
		stdout(&output),
		stderr(&output)
	);
	let spec: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("spec json");
	assert_eq!(
		spec["title"], "Fake RPC",
		"the title comes from leptoskit.toml"
	);
	assert_eq!(spec["version"], "4.5.6", "the version comes from the app");
	assert!(
		stderr(&output).contains("the spec has no functions"),
		"an empty spec gets a hint: {}",
		stderr(&output)
	);
}

#[test]
fn servers_that_do_not_export_are_explained() {
	let scratch = Scratch::new("no-export");
	fake_server(
		&scratch.path,
		"fake-server-silent",
		"\tprintln!(\"I never heard of the spec\");",
	);
	let output = leptoskit(&scratch.path, &["rpc", "list"]);
	assert!(!output.status.success());
	assert!(
		stderr(&output).contains("wrote no spec")
			&& stderr(&output).contains("export_if_requested"),
		"{}",
		stderr(&output)
	);

	fake_server(
		&scratch.path,
		"fake-server-crash",
		"\teprintln!(\"boom\");\n\tstd::process::exit(3);",
	);
	let output = leptoskit(&scratch.path, &["rpc", "list"]);
	assert!(!output.status.success());
	assert!(
		stderr(&output).contains("exited with") && stderr(&output).contains("boom"),
		"{}",
		stderr(&output)
	);
}

#[test]
fn servers_that_never_exit_are_explained() {
	let scratch = Scratch::new("export-timeout");
	// A server that never exits (it started serving) is killed after the
	// timeout, with the explanation and the override knob.
	fake_server(
		&scratch.path,
		"fake-server-hang",
		"\tstd::thread::sleep(std::time::Duration::from_secs(60));",
	);
	let output = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args(["rpc", "list"])
		.current_dir(&scratch.path)
		.env("LEPTOSKIT_RPC_EXPORT_TIMEOUT_SECS", "1")
		.output()
		.expect("run leptoskit");
	assert!(!output.status.success());
	assert!(
		stderr(&output).contains("did not exit after 1s")
			&& stderr(&output).contains("LEPTOSKIT_RPC_EXPORT_TIMEOUT_SECS"),
		"{}",
		stderr(&output)
	);
}

#[test]
fn servers_requiring_missing_features_are_explained() {
	let scratch = Scratch::new("export-required-features");
	// A binary cargo skips (missing required features) is explained too.
	fake_server(&scratch.path, "fake-server-gated", "");
	std::fs::write(
		scratch.path.join("Cargo.toml"),
		"[package]\nname = \"fake-server-gated\"\nversion = \"4.5.6\"\nedition = \
		 \"2021\"\n\n[[bin]]\nname = \"fake-server-gated\"\npath = \
		 \"src/main.rs\"\nrequired-features = [\"ssr\"]\n\n[features]\nssr = []\n\n[workspace]\n",
	)
	.expect("manifest");
	let output = leptoskit(&scratch.path, &["rpc", "list"]);
	assert!(!output.status.success());
	assert!(
		stderr(&output).contains("built no server binary"),
		"{}",
		stderr(&output)
	);
}

#[test]
fn library_only_apps_need_a_spec_file() {
	let scratch = Scratch::new("lib-only");
	std::fs::write(
		scratch.path.join("leptoskit.toml"),
		"[app]\nname = \"lib-only\"\ndefault_platform = \"server\"\n",
	)
	.expect("config");
	std::fs::write(
		scratch.path.join("Cargo.toml"),
		"[package]\nname = \"lib-only\"\nversion = \"0.1.0\"\nedition = \
		 \"2021\"\n\n[lib]\n\n[workspace]\n",
	)
	.expect("manifest");
	std::fs::create_dir_all(scratch.path.join("src")).expect("src");
	std::fs::write(scratch.path.join("src/lib.rs"), "").expect("lib");
	let output = leptoskit(&scratch.path, &["rpc", "list"]);
	assert!(!output.status.success());
	assert!(
		stderr(&output).contains("has no binary target") && stderr(&output).contains("--from"),
		"{}",
		stderr(&output)
	);
}

#[test]
fn specs_can_come_from_a_url() {
	let scratch = Scratch::new("url");
	project(&scratch.path, "");

	// A one-shot HTTP server for the fixture.
	let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
	let port = listener.local_addr().expect("addr").port();
	let served = std::thread::spawn(move || {
		use std::io::Read;
		use std::io::Write;
		let (mut stream, _) = listener.accept().expect("accept");
		let mut request = [0_u8; 4096];
		let _ = stream.read(&mut request);
		let response = format!(
			"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: \
			 {}\r\nConnection: close\r\n\r\n{SPEC}",
			SPEC.len()
		);
		stream.write_all(response.as_bytes()).expect("respond");
	});
	let url = format!("http://127.0.0.1:{port}/spec.json");
	let output = leptoskit(&scratch.path, &["rpc", "list", "--from", &url]);
	served.join().expect("server thread");

	if !output.status.success() && stderr(&output).contains("could not run curl") {
		eprintln!("skipping: curl is not installed");

		return;
	}

	assert!(output.status.success(), "{}", stderr(&output));
	assert!(
		stdout(&output).contains("list_todos"),
		"{}",
		stdout(&output)
	);

	// An unreachable URL is a clear error.
	let output = leptoskit(
		&scratch.path,
		&["rpc", "list", "--from", "http://127.0.0.1:1/spec.json"],
	);
	assert!(!output.status.success());
	assert!(
		stderr(&output).contains("could not fetch"),
		"{}",
		stderr(&output)
	);
}

/// A `cargo` stub on the PATH, so `--build` exercises its check without
/// compiling anything. It is a shell script, so only Unix tests use it.
#[cfg(unix)]
fn cargo_stub(dir: &Path, exit_success: bool) {
	let bin = dir.join("cargo");
	let body = if exit_success {
		"#!/bin/sh\nexit 0\n"
	} else {
		"#!/bin/sh\necho 'stub failure' >&2\nexit 1\n"
	};

	std::fs::write(&bin, body).expect("stub");
	make_executable(&bin);
}

#[cfg(unix)]
fn make_executable(path: &Path) {
	use std::os::unix::fs::PermissionsExt;
	let mut permissions = std::fs::metadata(path).expect("stat").permissions();
	permissions.set_mode(0o755);
	std::fs::set_permissions(path, permissions).expect("chmod");
}

/// The stub is a shell script; the lines it covers run on the Linux coverage
/// job, which is where they matter.
#[cfg(unix)]
#[test]
fn build_runs_cargo_check_on_generated_crates() {
	let scratch = Scratch::new("build-flag");
	project(&scratch.path, "");
	let title = "RPC\"\ntarget.'cfg(all())'.dependencies.injected = { path = 'injected' }\n#";
	let mut spec: serde_json::Value = serde_json::from_str(SPEC).expect("spec");
	spec["title"] = serde_json::json!(title);
	std::fs::write(scratch.path.join("spec.json"), spec.to_string()).expect("hostile spec");
	let stub_dir = scratch.path.join("bin");
	std::fs::create_dir_all(&stub_dir).expect("bin");
	cargo_stub(&stub_dir, true);
	let path = std::env::join_paths([
		stub_dir.as_os_str(),
		std::env::split_paths(&std::env::var_os("PATH").expect("PATH"))
			.next()
			.unwrap_or_default()
			.as_os_str(),
	])
	.expect("join paths");
	let output = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args([
			"rpc",
			"generate",
			"--from",
			"spec.json",
			"--target",
			"rust",
			"--target",
			"cli",
			"--build",
		])
		.current_dir(&scratch.path)
		.env("PATH", &path)
		.output()
		.expect("run leptoskit");
	assert!(output.status.success(), "{}", stderr(&output));

	for target in ["rust", "cli"] {
		let manifest: toml::Value = toml::from_str(&read(
			&scratch.path.join(format!("clients/{target}/Cargo.toml")),
		))
		.expect("generated manifest");
		assert!(
			manifest.get("target").is_none(),
			"metadata cannot add target dependencies"
		);
		let mut dependencies: Vec<&str> = manifest["dependencies"]
			.as_table()
			.expect("dependencies")
			.keys()
			.map(String::as_str)
			.collect();
		dependencies.sort_unstable();
		let expected = if target == "rust" {
			vec!["reqwest", "serde", "serde_json", "serde_qs", "thiserror"]
		} else {
			vec![
				"clap",
				"reqwest",
				"serde",
				"serde_json",
				"serde_qs",
				"thiserror",
				"tokio",
			]
		};
		assert_eq!(
			dependencies, expected,
			"only generator-owned dependencies reach cargo check"
		);
	}

	assert!(
		stdout(&output).contains("rust: checked"),
		"{}",
		stdout(&output)
	);
	assert!(
		stdout(&output).contains("cli: checked"),
		"{}",
		stdout(&output)
	);

	cargo_stub(&stub_dir, false);
	let output = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args([
			"rpc",
			"generate",
			"--from",
			"spec.json",
			"--target",
			"rust",
			"--build",
		])
		.current_dir(&scratch.path)
		.env("PATH", &path)
		.output()
		.expect("run leptoskit");
	assert!(!output.status.success());
	assert!(
		stderr(&output).contains("does not compile"),
		"{}",
		stderr(&output)
	);
}

fn assert_typescript_client(root: &Path) {
	let index = read(&root.join("clients/typescript/index.ts"));
	assert!(
		index.contains("/** Which todos to list. */\n\t\tfilter: TodoFilter;"),
		"argument docs land on the input object: {index}"
	);
	assert!(
		index.contains("/** How many to return. */\n\t\tlimit?: number | null"),
		"{index}"
	);
	assert!(
		index.contains("- `filter` — Which todos to list."),
		"the method doc lists the arguments: {index}"
	);
	assert!(index.contains("DEFAULT_SERVER_URL: string | undefined = \"https://todos.example\""));
	let package = read(&root.join("clients/typescript/package.json"));
	assert!(package.contains("\"name\": \"@acme/todos\""), "{package}");
	assert!(package.contains("\"version\": \"0.3.0\""), "{package}");
}

fn assert_cli_docs(root: &Path) {
	let cli = read(&root.join("clients/cli/src/main.rs"));
	assert!(cli.contains("ListTodos {"), "{cli}");
	assert!(cli.contains("/// [filter] Which todos to list."), "{cli}");
	assert!(
		cli.contains("/// Which todos to list.\n\t\t#[arg(long)]"),
		"argument docs reach the flag help: {cli}"
	);
	assert!(cli.contains("server_url: Option<String>"), "{cli}");
}
