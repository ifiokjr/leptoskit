//! Strict, backend-only packaging. Descriptors are operator evidence, not
//! attestations.
use std::fs;
use std::io::Read as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;

use crate::verify::build::Engine;
use crate::verify::build::{self};

#[derive(Debug, clap::Args)]
pub struct Args {
	#[command(flatten)]
	reproduction: build::ReproduceArgs,
	/// Digest-pinned runtime image (not a mutable tag).
	#[arg(long)]
	runtime_image: String,
	/// Independently selected SHA-256 of the runtime image's Config JSON.
	#[arg(long)]
	runtime_config_sha256: String,
	/// Local image tag. This command never pushes.
	#[arg(long)]
	tag: String,
	/// Inspect an existing local tag instead of packaging a new image.
	#[arg(long)]
	reuse_existing: bool,
	/// Explicit operator declaration that this backend exposes no RPC API.
	#[arg(long)]
	no_rpc: bool,
	/// New descriptor file, written only after image extraction succeeds.
	#[arg(long)]
	out: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CheckState {
	Absent,
	NotChecked,
	Unsupported,
	NotApplicable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Descriptor {
	format_version: u32,
	kind: String,
	inventory_sha256: String,
	recipe: crate::verify::BuildRecipe,
	executable_sha256: String,
	executable_mode: u32,
	pub(super) migrations: String,
	runtime_image: String,
	runtime_config_sha256: String,
	/// Engine content-addressed configuration ID, never a registry digest.
	pub(super) engine_image_id: String,
	image_config_sha256: String,
	platform: String,
	assets: CheckState,
	services: CheckState,
	rpc_contract: RpcContract,
	native_guest_compatibility: CheckState,
	runtime_attestation: CheckState,
}

/// Export proves only which contract these bytes emit, not compatibility or
/// that arbitrary application code obeys the early-export startup convention.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum RpcContract {
	Exported {
		sha256: String,
		compatibility: CheckState,
	},
	NotApplicable {
		declaration: String,
	},
}

fn rpc_contract(engine: Engine, image: &str, no_rpc: bool) -> Result<RpcContract> {
	if no_rpc {
		return Ok(RpcContract::NotApplicable {
			declaration: "operator_declares_no_rpc".into(),
		});
	}
	let dir = tempfile::tempdir()?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o777))?;
	}
	let mount = format!("type=bind,source={},target=/spec", dir.path().display());
	ensure!(
		!dir.path().to_string_lossy().contains(','),
		"invalid export mount"
	);
	let container = String::from_utf8(output(
		engine,
		&[
			"create",
			"--network=none",
			"--read-only",
			"--cap-drop=ALL",
			"--security-opt=no-new-privileges",
			"--pids-limit=32",
			"--memory=128m",
			"--user=65532:65532",
			"--mount",
			&mount,
			"--env",
			"LEPTOSKIT_RPC_SPEC_OUT=/spec/rpc.json",
			"--entrypoint=/app/backend",
			image,
		],
	)?)?
	.trim()
	.to_owned();
	let checked = (|| -> Result<RpcContract> {
		output(engine, &["start", &container])?;
		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
		loop {
			let states: Vec<Value> =
				serde_json::from_slice(&output(engine, &["inspect", &container])?)?;
			let state = &states.first().context("missing export container")?["State"];
			if state["Running"] == false {
				ensure!(
					state["ExitCode"] == 0,
					"RPC export failed before successful exit"
				);
				break;
			}
			ensure!(
				std::time::Instant::now() < deadline,
				"RPC export timed out; call export_if_requested before database/auth startup"
			);
			std::thread::sleep(std::time::Duration::from_millis(100));
		}
		let path = dir.path().join("rpc.json");
		let metadata = fs::symlink_metadata(&path).context(
			"executable did not export RPC spec; call export_if_requested first in main",
		)?;
		ensure!(
			metadata.is_file() && metadata.len() <= 16 * 1024 * 1024,
			"invalid RPC export file"
		);
		let bytes = fs::read(path)?;
		leptoskit_rpc_codegen::ApiSpec::from_json(std::str::from_utf8(&bytes)?)?;
		Ok(RpcContract::Exported {
			sha256: super::digest(&bytes),
			compatibility: CheckState::NotChecked,
		})
	})();
	let removed = output(engine, &["rm", "--force", &container]);
	let contract = checked?;
	removed?;
	Ok(contract)
}

fn hash_json(value: &Value) -> Result<String> {
	Ok(super::digest(&serde_json::to_vec(value)?))
}

fn output(engine: Engine, args: &[&str]) -> Result<Vec<u8>> {
	let output = Command::new(engine.program()).args(args).output()?;
	ensure!(
		output.status.success(),
		"container engine failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);
	Ok(output.stdout)
}

fn inspect(engine: Engine, image: &str) -> Result<Value> {
	let values: Vec<Value> =
		serde_json::from_slice(&output(engine, &["image", "inspect", image])?)?;
	values
		.into_iter()
		.next()
		.context("image inspection returned no image")
}

fn pinned(image: &str) -> bool {
	image.rsplit_once("@sha256:").is_some_and(|(name, hash)| {
		!name.is_empty()
			&& name
				.bytes()
				.all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
			&& hash.len() == 64
			&& hash
				.bytes()
				.all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
	})
}

fn backend_only(source: &Path, config: &LeptoskitConfig) -> Result<()> {
	ensure!(
		config.app.platforms == [Platform::Server] && config.build.client_package.is_none(),
		"verified packaging supports backend-only projects; website assets evidence is unsupported"
	);
	let raw: toml::Value = toml::from_str(&fs::read_to_string(source.join("leptoskit.toml"))?)?;
	fn services(value: &toml::Value) -> bool {
		match value {
			toml::Value::Table(table) => {
				table
					.iter()
					.any(|(key, value)| key == "services" || key == "service" || services(value))
			}
			toml::Value::Array(values) => values.iter().any(services),
			_ => false,
		}
	}
	ensure!(
		!services(&raw) && !source.join("services").exists(),
		"service-bearing releases need evidence for every executable and schema; unsupported"
	);
	Ok(())
}

/// Validate the extracted tar member rather than trusting image labels.
fn executable(bytes: &[u8], expected: &str) -> Result<()> {
	let mut archive = tar::Archive::new(bytes);
	let mut entries = archive.entries()?;
	let mut entry = entries.next().context("missing image executable")??;
	ensure!(
		entry.header().entry_type().is_file() && entry.header().mode()? == 0o755,
		"image executable must be a regular file with mode 0755"
	);
	let mut content = Vec::new();
	entry.read_to_end(&mut content)?;
	ensure!(
		super::digest(&content) == expected,
		"image executable bytes differ"
	);
	drop(entry);
	ensure!(
		entries.next().is_none(),
		"unexpected extra image executable entries"
	);
	Ok(())
}

fn validate_image(image: &Value, platform: &str, config: &Value) -> Result<()> {
	ensure!(
		format!(
			"{}/{}",
			image["Os"].as_str().unwrap_or(""),
			image["Architecture"].as_str().unwrap_or("")
		) == platform,
		"wrong image platform"
	);
	let mut actual = image["Config"].clone();
	let mut expected = config.clone();
	for value in [&mut actual, &mut expected] {
		if value["Cmd"].is_null() {
			value["Cmd"] = json!([]);
		}
	}
	ensure!(actual == expected, "produced image configuration differs");
	Ok(())
}

fn source_schema(bundle: &build::ReproducedBundle) -> Result<String> {
	let source = bundle.root().join("source");
	let config = LeptoskitConfig::load_from(&source)?;
	backend_only(&source, &config)?;
	ensure!(
		bundle.recipe.package == config.app.name,
		"recipe package differs from managed backend"
	);
	let path = &config.db.as_ref().context("missing database")?.migrations;
	ensure!(
		!path.is_absolute()
			&& path
				.components()
				.all(|c| matches!(c, std::path::Component::Normal(_))),
		"migrations must be inside the verified source"
	);
	super::build::fingerprint(&source, &config)
}

pub(super) fn run(args: &Args) -> Result<()> {
	ensure!(!args.out.exists(), "descriptor output already exists");
	ensure!(
		!args.reproduction.offline && args.reproduction.checkout.is_none(),
		"managed verified packaging requires full reproduction from the canonical repository"
	);
	ensure!(
		pinned(&args.runtime_image),
		"runtime image must be digest-pinned"
	);
	ensure!(
		!args.tag.starts_with('-') && !args.tag.is_empty(),
		"invalid local image tag"
	);
	let bundle = build::reproduce(&args.reproduction)?;
	ensure!(bundle.full, "full reproduction required");
	let migrations = source_schema(&bundle)?;
	let engine = Engine::detect(args.reproduction.engine)?;
	let platform = build::platform(&bundle.recipe.target)?;
	output(
		engine,
		&["pull", "--platform", platform, &args.runtime_image],
	)?;
	let runtime = inspect(engine, &args.runtime_image)?;
	ensure!(
		hash_json(&runtime["Config"])? == args.runtime_config_sha256,
		"runtime configuration pin differs"
	);
	let mut expected = runtime["Config"].clone();
	ensure!(
		expected["OnBuild"].as_array().is_none_or(Vec::is_empty),
		"runtime ONBUILD instructions are forbidden"
	);
	ensure!(
		expected["Volumes"].is_null(),
		"runtime volumes are unsupported"
	);
	// No application source, Dockerfile, build scripts, or credentials enter
	// this context.
	let context = tempfile::tempdir()?;
	let entries = crate::verify::walk(bundle.root())?;
	ensure!(
		matches!(
			entries.get("artifacts/backend"),
			Some(crate::verify::Entry::File {
				unix_mode: Some(0o755),
				..
			})
		),
		"verified executable must have mode 0755"
	);
	let backend = fs::read(bundle.root().join("artifacts/backend"))?;
	let backend_hash = super::digest(&backend);
	fs::write(context.path().join("backend"), &backend)?;
	let dockerfile = format!(
		"FROM {}\nCOPY --chmod=0755 backend /app/backend\nWORKDIR /app\nUSER \
		 65532:65532\nENTRYPOINT [\"/app/backend\"]\nCMD []\n",
		args.runtime_image
	);
	fs::write(context.path().join("Dockerfile"), dockerfile)?;
	expected["WorkingDir"] = json!("/app");
	expected["User"] = json!("65532:65532");
	expected["Entrypoint"] = json!(["/app/backend"]);
	expected["Cmd"] = json!([]);
	let label = format!("{}={migrations}", super::build::SCHEMA_LABEL);
	if expected["Labels"].is_null() {
		expected["Labels"] = json!({});
	}
	expected["Labels"][super::build::SCHEMA_LABEL] = json!(migrations);
	if !args.reuse_existing {
		output(
			engine,
			&[
				"build",
				"--network=none",
				"--platform",
				platform,
				"--label",
				&label,
				"--tag",
				&args.tag,
				context
					.path()
					.to_str()
					.context("context path is not UTF-8")?,
			],
		)?;
	}
	let image = inspect(engine, &args.tag)?;
	validate_image(&image, platform, &expected)?;
	let id = image["Id"].as_str().context("missing image ID")?;
	let container = String::from_utf8(output(engine, &["create", id])?)?
		.trim()
		.to_owned();
	let checked = (|| -> Result<()> {
		executable(
			&output(engine, &["cp", &format!("{container}:/app/backend"), "-"])?,
			&backend_hash,
		)?;
		// Reject additional application services inherited from the base.
		let exported = context.path().join("image.tar");
		output(
			engine,
			&[
				"export",
				"--output",
				exported.to_str().context("export path")?,
				&container,
			],
		)?;
		for entry in tar::Archive::new(fs::File::open(exported)?).entries()? {
			let entry = entry?;
			let path = entry.path()?;
			ensure!(
				!path
					.components()
					.filter(|c| !matches!(c, std::path::Component::CurDir))
					.collect::<PathBuf>()
					.starts_with("app/services"),
				"runtime contains unsupported application services"
			);
		}
		Ok(())
	})();
	let removed = output(engine, &["rm", &container]);
	checked?;
	removed?;
	let rpc_contract = rpc_contract(engine, id, args.no_rpc)?;
	let descriptor = Descriptor {
		format_version: 1,
		kind: "leptoskit-verified-backend-image".into(),
		inventory_sha256: bundle.inventory_sha256.clone(),
		recipe: bundle.recipe.clone(),
		executable_sha256: backend_hash,
		executable_mode: 0o755,
		migrations,
		runtime_image: args.runtime_image.clone(),
		runtime_config_sha256: args.runtime_config_sha256.clone(),
		engine_image_id: id.into(),
		image_config_sha256: hash_json(&image["Config"])?,
		platform: platform.into(),
		assets: CheckState::NotApplicable,
		services: CheckState::Unsupported,
		rpc_contract,
		native_guest_compatibility: CheckState::NotApplicable,
		runtime_attestation: CheckState::Absent,
	};
	let bytes = serde_json::to_vec_pretty(&descriptor)?;
	use std::io::Write as _;
	fs::OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(&args.out)?
		.write_all(&bytes)?;
	println!(
		"Packaged {id}; descriptor SHA-256 {}. No registry digest or runtime attestation.",
		super::digest(&bytes)
	);
	Ok(())
}

pub(super) fn read(path: &Path, pin: &str) -> Result<Descriptor> {
	let bytes = fs::read(path)?;
	ensure!(
		super::digest(&bytes) == pin,
		"descriptor does not match independently authenticated pin"
	);
	let descriptor: Descriptor = serde_json::from_slice(&bytes)?;
	descriptor.validate()?;
	Ok(descriptor)
}

impl Descriptor {
	fn validate(&self) -> Result<()> {
		ensure!(
			match &self.rpc_contract {
				RpcContract::Exported {
					sha256,
					compatibility,
				} =>
					sha256.len() == 64
						&& sha256.bytes().all(|b| b.is_ascii_hexdigit())
						&& *compatibility == CheckState::NotChecked,
				RpcContract::NotApplicable { declaration } =>
					declaration == "operator_declares_no_rpc",
			},
			"unsupported RPC evidence"
		);
		ensure!(
			self.format_version == 1
				&& self.kind == "leptoskit-verified-backend-image"
				&& self.assets == CheckState::NotApplicable
				&& self.services == CheckState::Unsupported
				&& self.native_guest_compatibility == CheckState::NotApplicable
				&& self.runtime_attestation == CheckState::Absent
				&& self.executable_mode == 0o755,
			"unsupported release evidence"
		);
		ensure!(
			self.engine_image_id
				.strip_prefix("sha256:")
				.is_some_and(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit())),
			"invalid engine image ID"
		);
		Ok(())
	}

	pub(super) fn check_remote(&self, host: &str, tag: &str, schema: &str) -> Result<()> {
		self.validate()?;
		ensure!(
			self.migrations == schema,
			"descriptor schema differs from release"
		);
		let bytes = super::remote::execute(
			host,
			&format!("docker image inspect {}", super::remote::quote(tag)),
			None,
		)?;
		let images: Vec<Value> = serde_json::from_slice(&bytes)?;
		let image = images.first().context("remote image absent")?;
		ensure!(
			image["Id"].as_str() == Some(&self.engine_image_id)
				&& hash_json(&image["Config"])? == self.image_config_sha256,
			"remote image differs from reproduced packaging evidence"
		);
		ensure!(
			format!(
				"{}/{}",
				image["Os"].as_str().unwrap_or(""),
				image["Architecture"].as_str().unwrap_or("")
			) == self.platform,
			"remote platform differs"
		);
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn archive(content: &[u8], mode: u32) -> Vec<u8> {
		let mut builder = tar::Builder::new(Vec::new());
		let mut header = tar::Header::new_gnu();
		header.set_size(content.len() as u64);
		header.set_mode(mode);
		header.set_cksum();
		builder
			.append_data(&mut header, "backend", content)
			.unwrap();
		builder.into_inner().unwrap()
	}

	#[test]
	fn image_bytes_and_mode_are_checked_without_trusting_labels() {
		let pin = super::super::digest(b"verified");
		assert!(executable(&archive(b"verified", 0o755), &pin).is_ok());
		assert!(executable(&archive(b"substituted", 0o755), &pin).is_err());
		assert!(executable(&archive(b"verified", 0o644), &pin).is_err());
	}

	#[test]
	fn image_platform_entrypoint_user_and_environment_are_bound() {
		let config = json!({"Entrypoint": ["/app/backend"], "Cmd": [], "User": "65532:65532", "Env": ["SAFE=1"]});
		let image = json!({"Os": "linux", "Architecture": "amd64", "Config": config});
		assert!(validate_image(&image, "linux/amd64", &config).is_ok());
		assert!(validate_image(&image, "linux/arm64", &config).is_err());
		for (key, replacement) in [
			("Entrypoint", json!(["/bin/sh"])),
			("User", json!("0")),
			("Env", json!(["SAFE=0"])),
		] {
			let mut changed = image.clone();
			changed["Config"][key] = replacement;
			assert!(validate_image(&changed, "linux/amd64", &config).is_err());
		}
	}

	#[test]
	fn runtime_tags_and_dockerfile_injection_are_rejected() {
		assert!(pinned(&format!(
			"registry/runtime@sha256:{}",
			"a".repeat(64)
		)));
		for value in [
			"runtime:latest",
			"runtime@sha256:abc",
			"runtime\nRUN evil@sha256:",
		] {
			assert!(!pinned(value));
		}
	}

	#[test]
	fn website_and_service_evidence_cannot_be_omitted() {
		let dir = tempfile::tempdir().unwrap();
		let raw = concat!(
			"[app]\nname='backend'\n",
			"platforms=['server']\ndefault_platform='server'\n",
			"render='ssr'\n",
		);
		fs::write(dir.path().join("leptoskit.toml"), raw).unwrap();
		let mut config = LeptoskitConfig::from_toml(raw, dir.path()).unwrap();
		assert!(backend_only(dir.path(), &config).is_ok());
		config.app.platforms = vec![Platform::Web];
		assert!(backend_only(dir.path(), &config).is_err());
		config.app.platforms = vec![Platform::Server];
		fs::create_dir(dir.path().join("services")).unwrap();
		assert!(backend_only(dir.path(), &config).is_err());
		fs::remove_dir(dir.path().join("services")).unwrap();
		fs::write(
			dir.path().join("leptoskit.toml"),
			format!("{raw}\n[services.worker]\npackage='worker'\n"),
		)
		.unwrap();
		assert!(backend_only(dir.path(), &config).is_err());
	}

	#[test]
	fn legacy_history_has_no_evidence() {
		let release: super::super::Release =
			serde_json::from_value(json!({"version":"old","migrations":"schema"})).unwrap();
		assert!(release.evidence.is_none());
	}

	#[test]
	fn replaced_descriptor_cannot_use_original_pin() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("descriptor.json");
		fs::write(&path, b"{}").unwrap();
		assert!(
			read(&path, &super::super::digest(b"original"))
				.unwrap_err()
				.to_string()
				.contains("pin")
		);
		assert!(read(&path, &super::super::digest(b"{}")).is_err());
	}
}
