//! Server export lifecycle and invocation-owned output.

use std::process::Child;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;
use leptoskit_config::LeptoskitConfig;
use leptoskit_rpc_codegen::ApiSpec;
use leptoskit_rpc_codegen::spec;

use crate::wasm;

const EXPORT_TIMEOUT: Duration = Duration::from_mins(5);
const EXPORT_TIMEOUT_ENV: &str = "LEPTOSKIT_RPC_EXPORT_TIMEOUT_SECS";
const POLL_INTERVAL: Duration = Duration::from_millis(50);

fn export_timeout() -> Duration {
	std::env::var(EXPORT_TIMEOUT_ENV)
		.ok()
		.and_then(|seconds| seconds.parse::<u64>().ok())
		.map_or(EXPORT_TIMEOUT, Duration::from_secs)
}

pub(super) fn export_from_server(config: &LeptoskitConfig, root: &Utf8PathBuf) -> Result<ApiSpec> {
	let binary = server_binary(config, root)?;
	let options = crate::build::server_options(config, false)?;
	// The directory owns only this invocation's output, while Cargo's target
	// directory remains shared for compilation. Keep it until the child is
	// joined.
	let output = tempfile::Builder::new()
		.prefix("leptoskit-fns-export-")
		.tempdir()
		.context("could not create a temporary spec export directory")?;
	let out = output.path().join("fns-spec.json");
	let title = config
		.rpc
		.title
		.clone()
		.unwrap_or_else(|| format!("{} RPC", config.app.name));
	let mut child = Command::new(&binary)
		.current_dir(root)
		.envs(&options.env)
		.env(spec::SPEC_OUT_ENV, &out)
		.env(spec::SPEC_TITLE_ENV, title)
		.env(spec::SPEC_VERSION_ENV, config.app.version.to_string())
		.env(
			spec::SPEC_DESCRIPTION_ENV,
			config.rpc.description.clone().unwrap_or_default(),
		)
		.env(spec::SPEC_SERVERS_ENV, config.rpc.servers.join(","))
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::piped())
		.spawn()
		.with_context(|| format!("could not run {}", binary.as_str()))?;
	let status = wait_for_export(&mut child, &binary)?;
	let stderr = read_stderr(&mut child)?;

	anyhow::ensure!(
		status.success(),
		"{} exited with {status} while exporting the spec:\n{}",
		binary.as_str(),
		stderr.trim()
	);
	let text = std::fs::read_to_string(&out).with_context(|| {
		format!(
			"the server exited but wrote no spec to {}. Call \
			 `leptoskit_rpc::spec::export_if_requested()` first thing in `main`.",
			out.display()
		)
	})?;
	ApiSpec::from_json(&text).context("reading the exported spec")
}

fn server_binary(config: &LeptoskitConfig, root: &Utf8PathBuf) -> Result<Utf8PathBuf> {
	let metadata = wasm::cargo_metadata(root.as_std_path())?;
	let package = metadata.package(&config.app.name).with_context(|| {
		format!(
			"app.name `{}` in leptoskit.toml must name the server package, but the cargo \
			 workspace has no such member",
			config.app.name
		)
	})?;
	package.bin_target().with_context(|| {
		format!(
			"`{}` has no binary target to export the spec from; pass --from <spec.json> instead",
			config.app.name
		)
	})?;
	let options = crate::build::server_options(config, false)?;
	let binary = crate::build::compile_server(root, &config.app.name, &options)?
		.with_context(|| format!("`{}` built no server binary", config.app.name))?;
	Utf8PathBuf::from_path_buf(binary)
		.map_err(|_| anyhow::anyhow!("the server binary path is not UTF-8"))
}

fn stop_child(child: &mut Child) -> Result<()> {
	let stopped = child.kill().context("stopping the server binary");
	let joined = child.wait().context("joining the stopped server binary");
	stopped?;
	joined?;
	Ok(())
}

fn wait_for_export(child: &mut Child, binary: &Utf8PathBuf) -> Result<ExitStatus> {
	let timeout = export_timeout();
	let started = Instant::now();

	loop {
		match child.try_wait() {
			Ok(Some(status)) => return Ok(status),
			Ok(None) => {}
			Err(error) => {
				stop_child(child)?;
				return Err(error).context("waiting for the server binary");
			}
		}

		if started.elapsed() > timeout {
			stop_child(child)?;
			anyhow::bail!(
				"{} did not exit after {}s. The spec is exported by \
				 `leptoskit_rpc::spec::export_if_requested()`, which must run first in `main`, \
				 before the server binds its port (set {EXPORT_TIMEOUT_ENV} to wait longer).",
				binary.as_str(),
				timeout.as_secs()
			);
		}

		std::thread::sleep(POLL_INTERVAL);
	}
}

fn read_stderr(child: &mut Child) -> Result<String> {
	use std::io::Read;
	let mut text = String::new();

	if let Some(mut stderr) = child.stderr.take() {
		stderr
			.read_to_string(&mut text)
			.context("reading server diagnostics")?;
	}

	Ok(text)
}
