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

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;
use leptoskit_config::LeptoskitConfig;
use leptoskit_rpc_codegen::ApiSpec;
use leptoskit_rpc_codegen::GenerateOptions;
use leptoskit_rpc_codegen::Scaffold;
use leptoskit_rpc_codegen::TargetLanguage;

use crate::cli::RpcCommand;
use crate::cli::SpecFormat;

mod export;
use export::export_from_server;

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
			write_spec(
				&config,
				&root,
				out,
				format,
				from.as_deref(),
				include_internal,
			)
		}
		RpcCommand::Check {
			against,
			from,
			include_internal,
		} => check(&config, &root, &against, from.as_deref(), include_internal),
		RpcCommand::List {
			from,
			include_internal,
		} => list_spec(&config, &root, from.as_deref(), include_internal),
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
			generate(
				&config,
				&root,
				GenerateRequest {
					target,
					out,
					package,
					server_url,
					from,
					include_internal,
					scaffold,
					build,
				},
			)
		}
	}
}

/// Serializes the visible specification and writes it to the selected
/// destination.
fn write_spec(
	config: &LeptoskitConfig,
	root: &Utf8PathBuf,
	out: Option<String>,
	format: SpecFormat,
	from: Option<&str>,
	include_internal: bool,
) -> Result<()> {
	let spec = acquire_spec(config, root, from)?;
	let spec = visible(config, &spec, include_internal);

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

			std::fs::write(&path, text).with_context(|| format!("could not write {path}"))?;
			println!("wrote {} function(s) to {path}", spec.functions.len());
		}
		None => print!("{text}"),
	}

	Ok(())
}

/// Fails on a change that would break a client generated from the previous
/// spec: a removed function, a changed kind or route, a new required
/// argument, a removed argument, a changed result schema, or a new required
/// scope.
fn check(
	config: &LeptoskitConfig,
	root: &Utf8PathBuf,
	against: &str,
	from: Option<&str>,
	include_internal: bool,
) -> Result<()> {
	let current = visible(config, &acquire_spec(config, root, from)?, include_internal);
	let previous = visible(
		config,
		&acquire_spec(config, root, Some(against))?,
		include_internal,
	);
	let breaks = breaking_changes(&previous, &current);

	if breaks.is_empty() {
		println!("no breaking changes");

		return Ok(());
	}

	for line in &breaks {
		println!("breaking: {line}");
	}

	anyhow::bail!(
		"{} breaking change(s) against {against}; bump the RPC's version or keep the old shape",
		breaks.len()
	)
}

/// The changes from `previous` to `current` an installed client would break
/// on.
fn breaking_changes(previous: &ApiSpec, current: &ApiSpec) -> Vec<String> {
	let mut breaks = Vec::new();

	for old in &previous.functions {
		let Some(new) = current
			.functions
			.iter()
			.find(|function| function.name == old.name)
		else {
			breaks.push(format!("removed `{}`", old.name));
			continue;
		};

		if old.kind != new.kind {
			breaks.push(format!(
				"`{}` changed from a {} to a {}",
				old.name,
				old.kind.as_str(),
				new.kind.as_str()
			));
		}

		if old.path != new.path || old.method != new.method {
			breaks.push(format!(
				"`{}` moved from {} {} to {} {}",
				old.name, old.method, old.path, new.method, new.path
			));
		}

		if old.input != new.input || old.output != new.output {
			breaks.push(format!("`{}` changed its encoding", old.name));
		}

		if old.output_schema != new.output_schema {
			breaks.push(format!("`{}` changed its result schema", old.name));
		}

		for param in &old.params {
			if !new
				.params
				.iter()
				.any(|candidate| candidate.name == param.name)
			{
				breaks.push(format!(
					"`{}` removed its `{}` argument",
					old.name, param.name
				));
			}
		}

		for param in &new.params {
			let was = old
				.params
				.iter()
				.find(|candidate| candidate.name == param.name);

			if was.is_none() && param.is_required() {
				breaks.push(format!(
					"`{}` added the required `{}` argument",
					old.name, param.name
				));
			}
		}

		for scope in &new.scopes {
			if !old.scopes.contains(scope) {
				breaks.push(format!("`{}` now requires the `{scope}` scope", old.name));
			}
		}
	}

	breaks
}

fn list_spec(
	config: &LeptoskitConfig,
	root: &Utf8PathBuf,
	from: Option<&str>,
	include_internal: bool,
) -> Result<()> {
	let spec = acquire_spec(config, root, from)?;
	let spec = visible(config, &spec, include_internal);

	if spec.functions.is_empty() {
		println!("no published functions; mark server functions with #[leptoskit_rpc::api]");

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

struct GenerateRequest {
	target: Vec<String>,
	out: Option<String>,
	package: Option<String>,
	server_url: Option<String>,
	from: Option<String>,
	include_internal: bool,
	scaffold: Option<String>,
	build: bool,
}

/// Resolves configured targets and preserves each target's scaffold policy.
fn generate(config: &LeptoskitConfig, root: &Utf8PathBuf, request: GenerateRequest) -> Result<()> {
	let GenerateRequest {
		target,
		out,
		package,
		server_url,
		from,
		include_internal,
		scaffold,
		build,
	} = request;
	let spec = acquire_spec(config, root, from.as_deref())?;
	let plan = plan_targets(config, &target, out.as_deref())?;
	let scaffold_override: Option<Scaffold> = scaffold.as_deref().map(str::parse).transpose()?;
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
		let (written, kept) = generate_target(root, &spec, &options, scaffold, build)?;
		written_total += written;

		kept_total += kept;
	}
	let kept = if kept_total == 0 {
		String::new()
	} else {
		format!(
			"; {kept_total} existing scaffold file(s) kept (pass --scaffold full to rewrite them)"
		)
	};

	println!(
		"generated {written_total} file(s) for {} function(s){kept}",
		visible(config, &spec, include_internal).functions.len()
	);
	Ok(())
}

/// The spec with internal functions and channels and `rpc.exclude` matches
/// removed, unless internal ones were asked for.
fn visible(config: &LeptoskitConfig, spec: &ApiSpec, include_internal: bool) -> ApiSpec {
	spec.published(
		include_internal || config.rpc.include_internal,
		&config.rpc.exclude,
	)
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
		crate::ui::warning(
			"the spec has no functions. Mark server functions with #[leptoskit_rpc::api], enable \
			 `leptoskit_rpc/ssr` in the server build, and make sure the server binary uses the \
			 crate that defines them.",
		);
	}

	Ok(spec)
}

fn generate_target(
	root: &Utf8PathBuf,
	spec: &ApiSpec,
	options: &GenerateOptions,
	scaffold: Scaffold,
	build: bool,
) -> Result<(usize, usize)> {
	let generated = spec.generate(options);
	let out_dir = root.join(options.out_dir.to_string_lossy().as_ref());
	// Generated crates carry their own `src/lib.rs`; writing into
	// the project root or the app's `src` would
	// clobber the app itself.
	anyhow::ensure!(
		out_dir != *root && out_dir != root.join("src"),
		"refusing to write the {} client into {}; set a different `out` for this target",
		options.language,
		out_dir
	);
	let report = generated
		.write_to(out_dir.as_std_path(), scaffold)
		.with_context(|| format!("could not write {out_dir}"))?;

	let kept = if report.kept.is_empty() {
		String::new()
	} else {
		format!(", {} kept", report.kept.len())
	};

	println!(
		"{}: {} file(s) in {}{kept}",
		options.language,
		report.written.len(),
		out_dir.as_str().to_owned()
	);

	if build && options.language.is_cargo() {
		let status = Command::new("cargo")
			.arg("check")
			.current_dir(&options.out_dir)
			.status()
			.context("could not run cargo")?;
		anyhow::ensure!(
			status.success(),
			"the generated {} crate does not compile",
			options.language
		);

		println!("{}: checked", options.language);
	}

	Ok((report.written.len(), report.kept.len()))
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

#[cfg(test)]
mod check_tests {
	use leptoskit_rpc_codegen::ApiFunction;
	use leptoskit_rpc_codegen::ApiKind;
	use leptoskit_rpc_codegen::ApiParam;
	use leptoskit_rpc_codegen::HttpInput;
	use leptoskit_rpc_codegen::HttpOutput;

	use super::breaking_changes;

	fn function(name: &str) -> ApiFunction {
		ApiFunction {
			name: name.to_owned(),
			kind: ApiKind::Query,
			module_path: "app".to_owned(),
			struct_name: "Op".to_owned(),
			docs: String::new(),
			path: format!("/api/{name}"),
			method: "POST".to_owned(),
			input: HttpInput::Json,
			output: HttpOutput::Json,
			error_content_type: "text/plain".to_owned(),
			params: vec![ApiParam {
				name: "limit".to_owned(),
				docs: String::new(),
				schema: serde_json::json!({ "type": "integer" }),
				required: true,
			}],
			output_schema: serde_json::json!({ "type": "array" }),
			internal: false,
			deprecated: false,
			tags: Vec::new(),
			scopes: Vec::new(),
		}
	}

	fn spec(functions: Vec<ApiFunction>) -> leptoskit_rpc_codegen::ApiSpec {
		let mut spec = leptoskit_rpc_codegen::ApiSpec::empty("App", "1.0.0");
		spec.functions = functions;
		spec
	}

	#[test]
	fn only_breaking_changes_are_reported() {
		let previous = spec(vec![function("notes")]);
		assert_eq!(breaking_changes(&previous, &previous), Vec::<String>::new());

		let mut renamed = function("notes");
		renamed.path = "/api/notes_v2".to_owned();
		assert_eq!(
			breaking_changes(&previous, &spec(vec![renamed])),
			["`notes` moved from POST /api/notes to POST /api/notes_v2"]
		);

		let mut added = function("notes");
		added.params.push(ApiParam {
			name: "starting".to_owned(),
			docs: String::new(),
			schema: serde_json::json!({ "type": "string" }),
			required: true,
		});
		assert_eq!(
			breaking_changes(&previous, &spec(vec![added])),
			["`notes` added the required `starting` argument"]
		);

		assert_eq!(
			breaking_changes(&previous, &spec(Vec::new())),
			["removed `notes`"]
		);
	}
}
