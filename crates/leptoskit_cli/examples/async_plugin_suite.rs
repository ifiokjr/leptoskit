//! Generates an app with HTTP/WebSocket checks in its Check files action.
//! Start `cargo run -p leptoskit_cli --example async_plugin_server`, then run
//! this example with `--cli target/debug/leptoskit --destination
//! target/async-app`. Arm64 Android emulators use `--target android --host
//! 10.0.2.2`; x86-64 emulators use `--target android-x86_64`.
//! After preparation, use `leptoskit native build <target>` in the
//! generated app.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
struct Args {
	#[arg(long)]
	cli: PathBuf,
	#[arg(long)]
	destination: PathBuf,
	#[arg(long, value_parser = ["macos", "ios-simulator", "android", "android-x86_64"], default_value = "macos")]
	target: String,
	#[arg(long, default_value = "127.0.0.1")]
	host: std::net::Ipv4Addr,
	#[arg(long, default_value_t = 48931)]
	port: u16,
}

fn main() -> Result<()> {
	let args = Args::parse();
	let cli = args.cli.canonicalize()?;
	let sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
		.join("../..")
		.canonicalize()?;
	let destination = std::path::absolute(args.destination)?;
	run(Command::new(&cli)
		.args(["generate", "app"])
		.arg(&destination)
		.args([
			"--name",
			"async_plugin_suite",
			"--platform",
			"native",
			"--sdk-path",
		])
		.arg(&sdk))?;
	let http = format!("http://{}:{}", args.host, args.port);
	let websocket = format!("ws://{}:{}", args.host, args.port);
	for (path, guest) in [("Cargo.toml", false), ("guest/Cargo.toml", true)] {
		edit(&destination.join(path), |manifest| {
			let dependencies = manifest
				.get_mut("dependencies")
				.and_then(toml::Value::as_table_mut)
				.context("missing dependencies")?;
			for package in ["leptoskit_plugin_http", "leptoskit_plugin_websocket"] {
				let mut dependency = toml::Table::new();
				dependency.insert(
					"path".into(),
					sdk.join("crates")
						.join(package)
						.to_str()
						.context("non-UTF8 SDK")?
						.into(),
				);
				dependency.insert("default-features".into(), false.into());
				if guest {
					dependency.insert("features".into(), vec!["guest"].into());
				}
				dependencies.insert(package.into(), dependency.into());
			}
			Ok(())
		})?;
	}
	edit(&destination.join("leptoskit.toml"), |manifest| {
		let native = manifest
			.get_mut("native")
			.and_then(toml::Value::as_table_mut)
			.context("missing native config")?;
		let plugins = native
			.get_mut("plugins")
			.and_then(toml::Value::as_array_mut)
			.context("missing plugins")?;
		plugins.extend(["http".into(), "websocket".into()]);
		let mut options = toml::Table::new();
		for (name, origin) in [("http", &http), ("websocket", &websocket)] {
			let mut scope = toml::Table::new();
			scope.insert("origins".into(), vec![origin.clone()].into());
			if name == "http" {
				scope.insert("max_body_bytes".into(), 8192.into());
			}
			options.insert(name.into(), scope.into());
		}
		native.insert("plugin-options".into(), options.into());
		Ok(())
	})?;
	let guest = destination.join("guest/src/lib.rs");
	let source = std::fs::read_to_string(&guest)?;
	let marker = "Ok(\"File roundtrip passed\".to_owned())";
	anyhow::ensure!(
		source.matches(marker).count() == 1,
		"scaffold file action changed"
	);
	let source = source.replacen(marker, &format!("async_plugins().await?;\n\t{marker}"), 1);
	std::fs::write(
		guest,
		format!(
			"{source}\nconst HTTP_ORIGIN: &str = {http:?};\nconst WS_ORIGIN: &str = \
			 {websocket:?};\n{}",
			include_str!("../tests/fixtures/native_plugin_suite/async_guest.rs")
		),
	)?;
	run(Command::new(cli)
		.current_dir(&destination)
		.args(["native", "prepare", &args.target]))?;
	println!(
		"Prepared {} with async checks in Check files",
		destination.display()
	);
	Ok(())
}

fn edit(path: &Path, update: impl FnOnce(&mut toml::Table) -> Result<()>) -> Result<()> {
	let mut document = toml::from_str(&std::fs::read_to_string(path)?)?;
	update(&mut document)?;
	std::fs::write(path, toml::to_string_pretty(&document)?)?;
	Ok(())
}

fn run(command: &mut Command) -> Result<()> {
	let status = command
		.status()
		.with_context(|| format!("start {command:?}"))?;
	anyhow::ensure!(status.success(), "{command:?} exited with {status}");
	Ok(())
}
