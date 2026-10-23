//! `leptoskit rpc`: the published RPC, described and turned into clients.
//!
//! The spec comes from the server binary itself: the CLI builds it, runs it
//! with `LEPTOSKIT_RPC_SPEC_OUT` set, and `leptoskit_rpc::spec::
//! export_if_requested` writes the spec and exits before the server would
//! bind a port. That keeps the description tied to the code that was just
//! compiled. `--from` skips the build and reads a spec file or URL instead.
//!
//! From the spec, `generate` writes typed clients that speak the RPC's own
//! protocol — TypeScript over `fetch`, Python over the standard library, Go
//! over `net/http`, Rust over `reqwest`, Dart over `package:http`, a clap
//! command line with one command per function — plus the OpenAPI 3.1
//! document. Function docs and `# Arguments` entries land in every client's
//! doc comments and `--help` texts; `#[api(internal)]` functions are left
//! out unless asked for.

use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;
use leptoskit_config::LeptoskitConfig;
use leptoskit_rpc_codegen::ApiSpec;
use leptoskit_rpc_codegen::GenerateOptions;
use leptoskit_rpc_codegen::Scaffold;
use leptoskit_rpc_codegen::TargetLanguage;
use leptoskit_rpc_codegen::spec;

use crate::cli::RpcCommand;
use crate::cli::SpecFormat;
use crate::wasm;

/// How long the server binary gets to write the spec before it is killed,
/// unless `LEPTOSKIT_RPC_EXPORT_TIMEOUT_SECS` says otherwise.
const EXPORT_TIMEOUT: Duration = Duration::from_mins(5);

/// Overrides [`EXPORT_TIMEOUT`], in seconds, for servers that take long to
/// reach `main` — or for tests that should not wait five minutes.
const EXPORT_TIMEOUT_ENV: &str = "LEPTOSKIT_RPC_EXPORT_TIMEOUT_SECS";

fn export_timeout() -> Duration {
	std::env::var(EXPORT_TIMEOUT_ENV)
		.ok()
		.and_then(|seconds| seconds.parse::<u64>().ok())
		.map_or(EXPORT_TIMEOUT, Duration::from_secs)
}

/// Runs a `leptoskit rpc` subcommand.
pub fn run(command: RpcCommand) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let root = Utf8PathBuf::from_path_buf(root)
		.map_err(|_| anyhow::anyhow!("project path is not UTF-8"))?;
	match command {
		RpcCommand::Spec {
			out,
			format,
			from,
			include_internal,
		} => {
			let spec = acquire_spec(&config, &root, from.as_deref())?;
			let spec = visible(&config, &spec, include_internal);
			let text = match format {
				SpecFormat::Json => spec.to_json().context("serialize the spec")? + "\n",
				SpecFormat::Openapi => {
					serde_json::to_string_pretty(&spec.to_openapi())
						.context("serialize the OpenAPI document")?
						+ "\n"
				}
				SpecFormat::OpenapiYaml => spec.to_openapi_yaml(),
			};
			match out {
				Some(out) => {
					let path = root.join(out);
					if let Some(parent) = path.parent() {
						std::fs::create_dir_all(parent)
							.with_context(|| format!("could not create {parent}"))?;
					}
					std::fs::write(&path, text)
						.with_context(|| format!("could not write {path}"))?;
					println!("wrote {} function(s) to {path}", spec.functions.len());
				}
				None => print!("{text}"),
			}
			Ok(())
		}
		RpcCommand::List {
			from,
			include_internal,
		} => {
			let spec = acquire_spec(&config, &root, from.as_deref())?;
			let spec = visible(&config, &spec, include_internal);
			if spec.functions.is_empty() {
				println!(
					"no published functions; mark server functions with #[leptoskit_rpc::api]"
				);
				return Ok(());
			}
			let name_width = spec
				.functions
				.iter()
				.map(|function| function.name.len())
				.max()
				.unwrap_or(0);
			let route_width = spec
				.functions
				.iter()
				.map(|function| function.method.len() + 1 + function.path.len())
				.max()
				.unwrap_or(0);
			for function in &spec.functions {
				let route = format!("{} {}", function.method, function.path);
				let summary = function.docs.lines().next().unwrap_or_default();
				let marker = if function.internal { " (internal)" } else { "" };
				println!(
					"{:<name_width$}  {route:<route_width$}  {summary}{marker}",
					function.name
				);
			}
			Ok(())
		}
		RpcCommand::Generate {
			target,
			out,
			package,
			server_url,
			from,
			include_internal,
			scaffold,
			build,
		} => {
			let spec = acquire_spec(&config, &root, from.as_deref())?;
			let plan = plan_targets(&config, &target, out.as_deref())?;
			let scaffold_override: Option<Scaffold> =
				scaffold.as_deref().map(str::parse).transpose()?;
			let include_internal = include_internal || config.rpc.include_internal;

			let mut written_total = 0;
			let mut kept_total = 0;
			for entry in &plan {
				let scaffold = match scaffold_override {
					Some(mode) => mode,
					None => {
						entry
							.scaffold
							.as_deref()
							.map(str::parse)
							.transpose()?
							.unwrap_or_default()
					}
				};
				let package_name = package
					.clone()
					.or_else(|| entry.package.clone())
					.unwrap_or_else(|| default_package_name(entry.language, &config.app.name));
				let options = GenerateOptions {
					language: entry.language,
					out_dir: entry.out_dir.clone(),
					package_name,
					version: Some(config.app.version.to_string()),
					server_url: server_url
						.clone()
						.or_else(|| entry.server_url.clone())
						.or_else(|| config.rpc.servers.first().cloned()),
					include_internal,
					exclude: config.rpc.exclude.clone(),
				};
				let generated = spec.generate(&options);
				let out_dir = root.join(entry.out_dir.to_string_lossy().as_ref());
				// Generated crates carry their own `src/lib.rs`; writing into
				// the project root or the app's `src` would
				// clobber the app itself.
				anyhow::ensure!(
					out_dir != root && out_dir != root.join("src"),
					"refusing to write the {} client into {}; set a different `out` for this \
					 target",
					entry.language,
					out_dir
				);
				let report = generated
					.write_to(out_dir.as_std_path(), scaffold)
					.with_context(|| format!("could not write {out_dir}"))?;
				written_total += report.written.len();
				kept_total += report.kept.len();
				let kept = if report.kept.is_empty() {
					String::new()
				} else {
					format!(", {} kept", report.kept.len())
				};
				println!(
					"{}: {} file(s) in {}{kept}",
					entry.language,
					report.written.len(),
					out_dir.as_str().to_owned()
				);
				if build && entry.language.is_cargo() {
					let status = Command::new("cargo")
						.arg("check")
						.current_dir(&entry.out_dir)
						.status()
						.context("could not run cargo")?;
					anyhow::ensure!(
						status.success(),
						"the generated {} crate does not compile",
						entry.language
					);
					println!("{}: checked", entry.language);
				}
			}
			let kept = if kept_total == 0 {
				String::new()
			} else {
				format!(
					"; {kept_total} existing scaffold file(s) kept (pass --scaffold full to \
					 rewrite them)"
				)
			};
			println!(
				"generated {written_total} file(s) for {} function(s){kept}",
				visible(&config, &spec, include_internal).functions.len()
			);
			Ok(())
		}
	}
}

/// The spec with internal functions and `rpc.exclude` matches removed, unless
/// internal functions were asked for.
fn visible(config: &LeptoskitConfig, spec: &ApiSpec, include_internal: bool) -> ApiSpec {
	let exclude = &config.rpc.exclude;
	let include_internal = include_internal || config.rpc.include_internal;
	spec.filtered(|function| {
		(include_internal || !function.internal)
			&& !exclude
				.iter()
				.any(|glob| spec::glob_matches(glob, &function.name))
	})
}

/// One target to generate, with its resolved output directory.
#[derive(Debug, Clone)]
struct PlannedTarget {
	language: TargetLanguage,
	out_dir: PathBuf,
	package: Option<String>,
	server_url: Option<String>,
	scaffold: Option<String>,
}

/// Resolves `--target`/`--out` against `rpc.targets`: explicit targets win,
/// then the configured ones, then every target into its default directory.
fn plan_targets(
	config: &LeptoskitConfig,
	targets: &[String],
	out: Option<&str>,
) -> Result<Vec<PlannedTarget>> {
	if !targets.is_empty() {
		anyhow::ensure!(
			out.is_none() || targets.len() == 1,
			"--out applies to one target; pass one --target with it"
		);
		return targets
			.iter()
			.map(|name| {
				let language: TargetLanguage = name.parse()?;
				let configured = config
					.rpc
					.targets
					.iter()
					.find(|target| target.language == language.as_str());
				Ok(PlannedTarget {
					language,
					out_dir: out
						.or_else(|| configured.and_then(|target| target.out.as_deref()))
						.map_or_else(|| default_out_dir(language), root_path),
					package: configured.and_then(|target| target.package.clone()),
					server_url: configured.and_then(|target| target.server_url.clone()),
					scaffold: configured.and_then(|target| target.scaffold.clone()),
				})
			})
			.collect();
	}
	if !config.rpc.targets.is_empty() {
		return config
			.rpc
			.targets
			.iter()
			.map(|target| {
				let language: TargetLanguage = target.language.parse()?;
				Ok(PlannedTarget {
					language,
					out_dir: target
						.out
						.as_deref()
						.map_or_else(|| default_out_dir(language), root_path),
					package: target.package.clone(),
					server_url: target.server_url.clone(),
					scaffold: target.scaffold.clone(),
				})
			})
			.collect();
	}
	Ok(TargetLanguage::ALL
		.iter()
		.map(|language| {
			PlannedTarget {
				language: *language,
				out_dir: default_out_dir(*language),
				package: None,
				server_url: None,
				scaffold: None,
			}
		})
		.collect())
}

fn root_path(dir: &str) -> PathBuf {
	PathBuf::from(dir)
}

fn default_out_dir(language: TargetLanguage) -> PathBuf {
	PathBuf::from(language.default_out_dir())
}

/// A package name for a client, from the app name.
fn default_package_name(language: TargetLanguage, app_name: &str) -> String {
	let snake = app_name.replace('-', "_");
	match language {
		TargetLanguage::Rust | TargetLanguage::Cli => format!("{snake}_client"),
		TargetLanguage::Dart | TargetLanguage::Python => snake,
		TargetLanguage::Go | TargetLanguage::TypeScript | TargetLanguage::OpenApi => {
			app_name.to_owned()
		}
	}
}

/// Reads the spec from `--from`, or exports it from the server binary.
fn acquire_spec(
	config: &LeptoskitConfig,
	root: &Utf8PathBuf,
	from: Option<&str>,
) -> Result<ApiSpec> {
	let spec = match from {
		Some(source) if source.starts_with("http://") || source.starts_with("https://") => {
			let output = Command::new("curl")
				.args(["--fail", "--silent", "--show-error", "--location", source])
				.output()
				.context("could not run curl to fetch the spec; install curl or pass a file")?;
			anyhow::ensure!(
				output.status.success(),
				"could not fetch {source}: {}",
				String::from_utf8_lossy(&output.stderr).trim()
			);
			let text = String::from_utf8(output.stdout).context("the spec is not UTF-8")?;
			ApiSpec::from_json(&text).with_context(|| format!("reading the spec from {source}"))?
		}
		Some(path) => {
			let path = root.join(path);
			let text =
				std::fs::read_to_string(&path).with_context(|| format!("could not read {path}"))?;
			ApiSpec::from_json(&text).with_context(|| format!("reading the spec from {path}"))?
		}
		None => export_from_server(config, root)?,
	};
	if spec.functions.is_empty() {
		eprintln!(
			"warning: the spec has no functions. Mark server functions with \
			 #[leptoskit_rpc::api], enable `leptoskit_rpc/ssr` in the server build, and make sure \
			 the server binary uses the crate that defines them."
		);
	}
	Ok(spec)
}

/// Builds the server binary and runs it with `LEPTOSKIT_RPC_SPEC_OUT` set.
fn export_from_server(config: &LeptoskitConfig, root: &Utf8PathBuf) -> Result<ApiSpec> {
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
	let binary = Utf8PathBuf::from_path_buf(binary)
		.map_err(|_| anyhow::anyhow!("the server binary path is not UTF-8"))?;
	let out = metadata
		.target_directory
		.join("leptoskit")
		.join("fns-spec.json");
	if let Some(parent) = out.parent() {
		std::fs::create_dir_all(parent)
			.with_context(|| format!("could not create {}", parent.display()))?;
	}
	let _ = std::fs::remove_file(&out);

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
	let timeout = export_timeout();
	let started = Instant::now();
	let status = loop {
		if let Some(status) = child.try_wait().context("waiting for the server binary")? {
			break status;
		}
		if started.elapsed() > timeout {
			let _ = child.kill();
			anyhow::bail!(
				"{} did not exit after {}s. The spec is exported by \
				 `leptoskit_rpc::spec::export_if_requested()`, which must run first in `main`, \
				 before the server binds its port (set {EXPORT_TIMEOUT_ENV} to wait longer).",
				binary.as_str(),
				timeout.as_secs()
			);
		}
		std::thread::sleep(Duration::from_millis(50));
	};
	let stderr = child
		.stderr
		.take()
		.map(|mut stderr| {
			use std::io::Read;
			let mut text = String::new();
			let _ = stderr.read_to_string(&mut text);
			text
		})
		.unwrap_or_default();
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn package_names_follow_the_app() {
		assert_eq!(
			default_package_name(TargetLanguage::Rust, "my-app"),
			"my_app_client"
		);
		assert_eq!(
			default_package_name(TargetLanguage::Cli, "my-app"),
			"my_app_client"
		);
		assert_eq!(
			default_package_name(TargetLanguage::Dart, "my-app"),
			"my_app"
		);
		assert_eq!(default_package_name(TargetLanguage::Go, "my-app"), "my-app");
	}
}
