//! `leptoskit doctor`: checks the environment for the tooling the CLI runs.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;
use leptoskit_config::ConfigError;
use leptoskit_config::LeptoskitConfig;

use crate::ui::Outcome;
use crate::wasm::BindgenCompatibility;
use crate::wasm::WASM_TARGET;

/// One environment check.
struct Check {
	name: &'static str,
	/// Whether a failure stops builds (as opposed to some builds).
	required: bool,
	/// The detail line on success, or what is wrong.
	outcome: Result<String, String>,
}

/// Runs the checks and prints a report.
///
/// # Errors
///
/// Returns an error when any required check fails, so scripts and CI can
/// gate on the exit status.
pub fn run() -> Result<()> {
	let mut checks = vec![
		Check {
			name: "cargo",
			required: true,
			outcome: version_of("cargo"),
		},
		Check {
			name: "rustc",
			required: true,
			outcome: version_of("rustc"),
		},
		Check {
			name: WASM_TARGET,
			required: true,
			outcome: wasm_target_installed(),
		},
		Check {
			name: "wasm-opt",
			required: false,
			outcome: version_of("wasm-opt").map_err(|error| {
				format!("{error}; release builds need it unless `bundle.wasm_opt = false`")
			}),
		},
	];
	if let Some(check) = project_check() {
		checks.push(check);
	}
	if let Some(check) = extension_check() {
		checks.push(check);
	}
	if let Some(check) = devtools_check() {
		checks.push(check);
	}

	crate::ui::heading("doctor", "the tools leptoskit builds with");
	let mut summary = crate::ui::Summary::default();
	let mut failed = 0;
	for check in &checks {
		let (outcome, details) = match &check.outcome {
			Ok(details) => (Outcome::Passed, details.as_str()),
			Err(problem) if check.required => {
				failed += 1;
				(Outcome::Failed, problem.as_str())
			}
			Err(problem) => (Outcome::Warned, problem.as_str()),
		};
		summary.row(outcome, check.name, details, None);
	}
	summary.print();
	let until = if failed == 1 {
		"it passes"
	} else {
		"they pass"
	};
	anyhow::ensure!(
		failed == 0,
		"{} failed; builds will not work until {until}",
		crate::ui::count(failed, "required check"),
	);
	Ok(())
}

/// The first line a tool prints for `--version`, requiring a zero exit.
fn version_of(program: &str) -> Result<String, String> {
	let output = Command::new(program)
		.arg("--version")
		.output()
		.map_err(|error| format!("missing: {error}"))?;
	if !output.status.success() {
		return Err(format!(
			"`{program} --version` exited with {}",
			output.status
		));
	}
	Ok(String::from_utf8_lossy(&output.stdout)
		.lines()
		.next()
		.unwrap_or_default()
		.to_owned())
}

/// Whether the active toolchain's sysroot carries the wasm target's standard
/// library. Asking the sysroot (rather than rustup) also works for
/// toolchains rustup does not manage, such as Nix-provided ones.
fn wasm_target_installed() -> Result<String, String> {
	let output = Command::new("rustc")
		.args(["--print", "sysroot"])
		.output()
		.map_err(|error| format!("could not run rustc: {error}"))?;
	if !output.status.success() {
		return Err(format!(
			"`rustc --print sysroot` exited with {}",
			output.status
		));
	}
	let sysroot = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
	let target_lib = sysroot.join("lib").join("rustlib").join(WASM_TARGET);
	if target_lib.is_dir() {
		Ok("installed".to_owned())
	} else {
		Err(format!(
			"missing; install it with `rustup target add {WASM_TARGET}`"
		))
	}
}

/// Chrome for Testing, which `leptoskit dev` and `leptoskit test extension`
/// run extensions in, when the project has one.
fn extension_check() -> Option<Check> {
	let (config, _) = LeptoskitConfig::load().ok()?;
	config.extension.as_ref()?;
	Some(Check {
		name: "Chrome for Testing",
		required: false,
		outcome: crate::extension::dev::chrome()
			.map(|path| path.display().to_string())
			.ok_or_else(|| crate::extension::dev::support().err().unwrap_or_default()),
	})
}

/// Whether the app's client crate can take the `DevTools` agents, which
/// `leptoskit dev` turns on through a `devtools` feature.
fn devtools_check() -> Option<Check> {
	let (config, root) = LeptoskitConfig::load().ok()?;
	let metadata = crate::wasm::cargo_metadata(&root).ok()?;
	let client = crate::wasm::detect_client_with(
		config.build.client_package.as_deref(),
		Some(&config.app.name),
		&metadata,
	)
	.ok()??;
	let package = metadata.package(&client.name)?;
	Some(Check {
		name: "devtools",
		required: false,
		outcome: devtools_outcome(package),
	})
}

fn devtools_outcome(package: &crate::wasm::MetadataPackage) -> Result<String, String> {
	if package.declares_feature("devtools") {
		Ok(format!("{} declares `devtools`", package.name))
	} else {
		Err(format!(
			"add `devtools = [\"leptoskit/devtools\"]` to {}'s [features] so `leptoskit dev` can \
			 inspect it",
			package.name
		))
	}
}

/// The project's wasm-bindgen compatibility, when run inside a project.
fn project_check() -> Option<Check> {
	let outcome = match LeptoskitConfig::load() {
		Err(ConfigError::NotFound) => return None,
		Err(error) => Err(format!("{:#}", anyhow::Error::from(error))),
		Ok((_, root)) => {
			match find_lockfile(&root) {
				None => Ok("no Cargo.lock yet; `leptoskit build` creates one".to_owned()),
				Some(lockfile) => lockfile_compatibility(&lockfile),
			}
		}
	};
	Some(Check {
		name: "project wasm-bindgen",
		required: true,
		outcome,
	})
}

fn lockfile_compatibility(lockfile: &Path) -> Result<String, String> {
	let contents = std::fs::read_to_string(lockfile)
		.map_err(|error| format!("could not read {}: {error}", lockfile.display()))?;
	match crate::wasm::wasm_bindgen_compatibility(&contents) {
		Ok(BindgenCompatibility::NotLocked) => Ok("no wasm-bindgen in Cargo.lock".to_owned()),
		Ok(BindgenCompatibility::Compatible) => {
			Ok(format!(
				"binds with the embedded wasm-bindgen {}",
				crate::wasm::embedded_wasm_bindgen()
			))
		}
		Ok(BindgenCompatibility::Mismatch { locked }) => {
			Err(crate::wasm::mismatch_message(&locked))
		}
		Err(error) => Err(format!("{error:#}")),
	}
}

/// The nearest `Cargo.lock` at or above the project root: cargo keeps it at
/// the workspace root.
fn find_lockfile(root: &Path) -> Option<PathBuf> {
	root.ancestors()
		.map(|dir| dir.join("Cargo.lock"))
		.find(|path| path.is_file())
}
