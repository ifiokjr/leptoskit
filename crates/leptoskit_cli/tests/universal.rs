//! Compiled verification of one generated application across every destination.

use std::fs;
use std::io::Read;
use std::io::Write;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::TcpStream;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use leptoskit_cli::server::BuiltServer;
use leptoskit_cli::wasm;
use leptoskit_cli::wasm::CargoOptions;
use leptoskit_native::EventOutcome;
use leptoskit_native::Mutation;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_wasm::WasmHost;

pub mod support;

use support::Scratch;

/// Runs fixture builds without leaking host coverage instrumentation into Wasm.
fn command(project: &Path, program: &str, arguments: &[&str]) -> Output {
	Command::new(program)
		.args(arguments)
		.current_dir(project)
		.env_remove("RUSTFLAGS")
		.env("CARGO_TARGET_DIR", project.join("target"))
		.output()
		.expect("run generated application command")
}

/// Includes both Cargo output streams when a generated application fails to
/// build.
fn assert_success(output: &Output) {
	assert!(
		output.status.success(),
		"stdout:\n{}\nstderr:\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr),
	);
}

/// Adds local fixtures without introducing new downloads to the dependency
/// test.
fn add_probe(project: &Path, name: &str, source: &str) {
	let directory = project.join(name);
	fs::create_dir_all(directory.join("src")).expect("probe source directory");
	fs::write(
		directory.join("Cargo.toml"),
		format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
	)
	.expect("probe manifest");
	fs::write(directory.join("src/lib.rs"), source).expect("probe source");
	let manifest = project.join("Cargo.toml");
	let mut workspace: toml::Table =
		toml::from_str(&fs::read_to_string(&manifest).expect("workspace manifest"))
			.expect("parse workspace manifest");
	workspace["workspace"]["members"]
		.as_array_mut()
		.expect("workspace members")
		.push(name.into());
	fs::write(
		manifest,
		toml::to_string(&workspace).expect("encode workspace"),
	)
	.expect("write workspace manifest");
}

/// Gives the iOS override a dependency that cannot be used accidentally
/// elsewhere.
fn install_dependency_probes(project: &Path) {
	add_probe(
		project,
		"ios_probe",
		"pub fn heading() -> &'static str { \"Hello from iOS\" }\n",
	);
	add_probe(
		project,
		"server_only_probe",
		"#[cfg(target_arch = \"wasm32\")]\ncompile_error!(\"backend dependency leaked into the \
		 guest\");\n",
	);
	let manifest = project.join("counter_app/Cargo.toml");
	let mut app: toml::Table =
		toml::from_str(&fs::read_to_string(&manifest).expect("app manifest"))
			.expect("parse app manifest");
	app["dependencies"]
		.as_table_mut()
		.expect("app dependencies")
		.insert(
			"ios_probe".to_owned(),
			toml::Value::Table(toml::Table::from_iter([
				("path".to_owned(), "../ios_probe".into()),
				("optional".to_owned(), true.into()),
			])),
		);
	app["features"]["ios"]
		.as_array_mut()
		.expect("iOS feature dependencies")
		.push("dep:ios_probe".into());
	fs::write(
		manifest,
		toml::to_string(&app).expect("encode app manifest"),
	)
	.expect("write app manifest");
	let manifest = project.join("counter_backend/Cargo.toml");
	let mut backend = fs::read_to_string(&manifest).expect("backend manifest");
	backend.push_str("server_only_probe = { path = \"../server_only_probe\" }\n");
	fs::write(manifest, backend).expect("write backend manifest");
}

/// Reads Cargo's selected dependency graph, rather than all lockfile packages.
fn dependency_graph(project: &Path, platform: &str) -> String {
	let output = command(
		project,
		"cargo",
		&[
			"tree",
			"--package",
			"counter_app",
			"--target",
			"wasm32-unknown-unknown",
			"--no-default-features",
			"--features",
			platform,
			"--edges",
			"normal",
			"--prefix",
			"none",
		],
	);
	assert_success(&output);
	String::from_utf8(output.stdout).expect("UTF-8 dependency graph")
}

/// Uses the pipeline's Cargo handling with separate artifacts for each
/// platform.
fn compile_guest(project: &Path, platform: &str) -> PathBuf {
	let client = wasm::detect_client(project, Some("counter_app"))
		.expect("detect portable application")
		.expect("application cdylib");
	let platform = platform.parse().expect("concrete application platform");
	let mut options = CargoOptions::default()
		.with_platform(platform)
		.expect("select one platform");
	options.target_directory = Some(
		project
			.join(".leptoskit")
			.join(platform.to_string())
			.join("target"),
	);
	wasm::compile_wasm(project, &client, &options).expect("compile portable native guest")
}

/// Starts a guest and presses its actual committed button, including after
/// resize.
fn assert_counter(wasm: &Path, heading: &str) {
	let bytes = fs::read(wasm).expect("native guest Wasm");
	let mut host = WasmHost::headless(&bytes).expect("instantiate portable native application");
	host.start(390.0, 844.0).expect("mount shared application");
	let mutations = host.take_mutations();
	let button = mutations
		.iter()
		.find_map(|mutation| {
			match mutation {
				Mutation::Create {
					id,
					kind: NativeKind::Button,
					props,
				} if props.text.as_deref() == Some("Add one") => Some(*id),
				_ => None,
			}
		})
		.expect("the shared screen has an Add one button");
	let labels: Vec<_> = mutations
		.iter()
		.filter_map(|mutation| {
			match mutation {
				Mutation::Create { props, .. } => props.text.as_deref(),
				_ => None,
			}
		})
		.collect();
	assert!(labels.contains(&heading), "initial labels: {labels:?}");
	assert!(labels.contains(&"Count: 0"), "initial labels: {labels:?}");

	for count in 1..=3 {
		assert_eq!(
			host.dispatch(button, &NativeEvent::Press)
				.expect("press shared button"),
			EventOutcome::Handled
		);
		let mutations = host.take_mutations();
		assert!(
			mutations.iter().any(|mutation| {
				match mutation {
					Mutation::UpdateProps { props, .. } => {
						props.text.as_deref() == Some(format!("Count: {count}").as_str())
					}
					_ => false,
				}
			}),
			"counter update {count}: {mutations:?}"
		);
	}

	host.set_viewport(844.0, 390.0);
	assert_eq!(
		host.dispatch(button, &NativeEvent::Press)
			.expect("press after viewport change"),
		EventOutcome::Handled
	);
	assert!(host.take_mutations().iter().any(|mutation| {
		match mutation {
			Mutation::UpdateProps { props, .. } => props.text.as_deref() == Some("Count: 4"),
			_ => false,
		}
	}));
	let mut restarted = WasmHost::headless(&bytes).expect("restart application");
	restarted
		.start(390.0, 844.0)
		.expect("mount restarted application");
	assert!(restarted.take_mutations().iter().any(|mutation| {
		match mutation {
			Mutation::Create { props, .. } => props.text.as_deref() == Some("Count: 0"),
			_ => false,
		}
	}));
}

/// Stops the generated server even when an assertion fails.
struct RunningServer(Child);

impl Drop for RunningServer {
	fn drop(&mut self) {
		let _ = self.0.kill();
		let _ = self.0.wait();
	}
}

/// Sends a bounded request so a broken generated server cannot hang the suite.
fn request(address: SocketAddr, path: &str) -> String {
	let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
		.expect("connect generated server");
	stream
		.set_read_timeout(Some(Duration::from_secs(5)))
		.expect("HTTP read timeout");
	stream
		.set_write_timeout(Some(Duration::from_secs(5)))
		.expect("HTTP write timeout");
	write!(
		stream,
		"GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
	)
	.expect("HTTP request");
	let mut response = String::new();
	stream.read_to_string(&mut response).expect("HTTP response");
	response
}

/// Checks actual SSR and backend responses from the generated server binary.
fn assert_server(project: &Path) {
	let binary = BuiltServer::load(&project.join("target"), project).expect("built SSR binary");
	let reservation = TcpListener::bind("127.0.0.1:0").expect("reserve server address");
	let address = reservation.local_addr().expect("server address");
	drop(reservation);
	let log = fs::File::create(project.join("server.log")).expect("server log");
	let mut server = RunningServer(
		Command::new(binary.binary)
			.current_dir(project)
			.env("LEPTOS_OUTPUT_NAME", "counter_app")
			.env("LEPTOS_SITE_ADDR", address.to_string())
			.env("LEPTOS_SITE_ROOT", project.join("dist"))
			.stdout(Stdio::from(log.try_clone().expect("clone server log")))
			.stderr(Stdio::from(log))
			.spawn()
			.expect("start generated server"),
	);
	let deadline = Instant::now() + Duration::from_secs(15);

	while TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err() {
		assert!(
			server
				.0
				.try_wait()
				.expect("server process status")
				.is_none(),
			"server exited: {}",
			fs::read_to_string(project.join("server.log")).expect("read server log")
		);
		assert!(Instant::now() < deadline, "generated server did not start");
		std::thread::sleep(Duration::from_millis(25));
	}

	let page = request(address, "/");
	assert!(
		page.starts_with("HTTP/1.1 200"),
		"response: {page:?}; server log: {}",
		fs::read_to_string(project.join("server.log")).expect("read server log")
	);
	assert!(page.contains("Hello, leptoskit"), "{page}");
	assert!(page.contains("Count: 0"), "{page}");
	assert!(page.contains("counter_app.js"), "{page}");
	assert!(request(address, "/api/health").contains(r#"{"ready":true}"#));
}

/// Builds real generated code and executes the shared counter through both
/// hosts.
///
/// No device SDK is needed: native app code targets Wasm for every device.
#[test]
#[ignore = "builds SSR and six Wasm destinations; run by the `coverage` CI job"]
fn generated_application_shares_behavior_and_selects_platform_overrides() {
	let scratch = Scratch::new("universal-runtime");
	let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
	let created = command(
		&scratch.path,
		env!("CARGO_BIN_EXE_leptoskit"),
		&[
			"new",
			"counter",
			"--sdk-path",
			sdk.to_str().expect("SDK path"),
		],
	);
	assert_success(&created);
	let project = scratch.path.join("counter");
	install_dependency_probes(&project);
	let web = command(
		&project,
		env!("CARGO_BIN_EXE_leptoskit"),
		&["build", "--platform", "web"],
	);
	assert_success(&web);
	assert!(project.join("dist/pkg/counter_app.js").is_file());
	assert!(project.join("dist/pkg/counter_app_bg.wasm").is_file());
	let glue =
		fs::read_to_string(project.join("dist/pkg/counter_app.js")).expect("browser bindings");
	assert!(
		glue.contains("export function hydrate("),
		"Leptos HydrationScripts requires a named hydrate export"
	);
	assert_server(&project);
	assert!(!dependency_graph(&project, "web").contains("server_only_probe"));

	for platform in ["ios", "android", "macos", "windows", "linux"] {
		let wasm = compile_guest(&project, platform);
		assert_counter(&wasm, "Hello, leptoskit");
		let graph = dependency_graph(&project, platform);
		assert!(!graph.contains("counter_backend"), "{platform}: {graph}");
		assert!(!graph.contains("server_only_probe"), "{platform}: {graph}");
		assert_eq!(
			graph.contains("ios_probe"),
			platform == "ios",
			"{platform}: {graph}"
		);
	}

	let shared = fs::read_to_string(project.join("counter_app/src/routes/home.rs"))
		.expect("shared home screen");
	let override_file = project.join("counter_app/src/routes/home.ios.rs");
	fs::write(
		&override_file,
		shared.replace("Text(\"Hello, leptoskit\")", "Text(ios_probe::heading())"),
	)
	.expect("add iOS override after first build");
	assert_counter(&compile_guest(&project, "ios"), "Hello from iOS");
	assert_counter(&compile_guest(&project, "android"), "Hello, leptoskit");
	fs::remove_file(override_file).expect("remove iOS override");
	assert_counter(&compile_guest(&project, "ios"), "Hello, leptoskit");
	let conflicting = command(
		&project,
		"cargo",
		&[
			"check",
			"--package",
			"counter_app",
			"--target",
			"wasm32-unknown-unknown",
			"--no-default-features",
			"--features",
			"ios,android",
		],
	);
	assert!(
		!conflicting.status.success(),
		"ambiguous device features must fail"
	);
	assert!(
		String::from_utf8_lossy(&conflicting.stderr)
			.contains("platform features are mutually exclusive"),
		"{}",
		String::from_utf8_lossy(&conflicting.stderr)
	);
}
