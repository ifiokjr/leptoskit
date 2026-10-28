//! Generates independent runtime plugin crates using the same no-clobber rules
//! as application scaffolding.

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;

use crate::new_project::ensure_package_name;
use crate::new_project::write_new;

/// Creates a plugin with an implemented echo method and a behavior test.
///
/// `bridge_path` points to a local SDK crate during development;
/// otherwise the generated crate depends on this CLI's release version.
pub fn plugin(path: &str, name: Option<&str>, bridge_path: Option<&str>) -> Result<()> {
	let destination = std::path::Path::new(path);
	let name = name
		.or_else(|| destination.file_name().and_then(|name| name.to_str()))
		.context("pass --name when the destination has no package name")?;
	ensure_package_name(name)?;
	anyhow::ensure!(
		name.bytes().all(|byte| {
			byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
		}),
		"plugin names must use lowercase ASCII letters, digits, `_`, or `-`"
	);
	let crate_name = format!("leptoskit_plugin_{}", name.replace('-', "_"));
	anyhow::ensure!(
		crate_name.len() <= 64,
		"the generated crate name must be at most 64 bytes"
	);
	let mut dependency = toml::Table::new();
	dependency.insert("version".to_owned(), env!("CARGO_PKG_VERSION").into());
	if let Some(path) = bridge_path {
		let path = std::fs::canonicalize(path).context("resolve --bridge-path")?;
		anyhow::ensure!(
			path.join("Cargo.toml").is_file(),
			"--bridge-path must contain Cargo.toml"
		);
		let path = path.to_str().context("--bridge-path is not UTF-8")?;
		dependency.insert("path".to_owned(), path.into());
	}
	let manifest = format!(
		"[package]\nname = {crate_name:?}\nversion = \"0.1.0\"\nedition = \"2024\"\nlicense = \
		 \"Unlicense\"\n\n[package.metadata.leptoskit.runtime]\nname = {name:?}\nrust-type = \
		 \"Plugin\"\n\n[workspace]\n\n[dependencies.leptoskit_bridge]\n{}",
		toml::to_string(&dependency)?
	);
	let source = PLUGIN.replace("__PLUGIN_NAME__", name);
	let readme = format!(
		"# {crate_name}\n\nRun `cargo test`. Add this crate as a direct app dependency and \
		 `{name}` to `native.plugins`, then run `leptoskit native prepare` for your target. The \
		 echo method returns the same owned buffer without copying it.\n\nAdd Swift handlers \
		 under `apple/` using `LeptosKitBridge.NativePlugin` and Kotlin handlers under `android/` \
		 using `dev.leptoskit.bridge.NativePlugin` when this plugin needs native APIs. Browser \
		 implementations belong behind a wasm target configuration. Update the runtime metadata \
		 to select the native types instead of `rust-type` when using platform handlers.\n"
	);
	let files = [
		("Cargo.toml", manifest),
		("src/lib.rs", source),
		("readme.md", readme),
		(".gitignore", "/target\n/.build\n".to_owned()),
	];
	let dir = scaffold(destination, &files)?;
	println!("created {crate_name} in {dir}");
	println!("next: cd {dir} && cargo test");
	Ok(())
}

/// Creates a tooling plugin: a program answering the process protocol, one
/// invocation per hook.
///
/// # Errors
///
/// Returns an error for an invalid name or an existing file.
pub fn tool_plugin(path: &str, name: Option<&str>) -> Result<()> {
	let destination = std::path::Path::new(path);
	let name = name
		.or_else(|| destination.file_name().and_then(|name| name.to_str()))
		.context("pass --name when the destination has no package name")?;
	ensure_package_name(name)?;
	let program = format!("leptoskit-plugin-{}", name.replace('_', "-"));
	let manifest = format!(
		"[package]\nname = {program:?}\nversion = \"0.1.0\"\nedition = \"2024\"\nlicense = \
		 \"Unlicense\"\n\n[workspace]\n\n[dependencies]\nserde_json = \"1\"\n\n[[bin]]\nname = \
		 {program:?}\npath = \"src/main.rs\"\n"
	);
	let readme = format!(
		"# {program}\n\nA leptoskit tooling plugin: a program leptoskit runs once per hook, with \
		 a JSON envelope on standard input and a JSON reply on standard output. Start it with \
		 `plugin::{name}` commands after declaring it:\n\n```toml\n[[plugins]]\nname = \
		 {name:?}\ncommand = {program:?}\n```\n\nInstall it on PATH with `cargo install --path .` \
		 and leptoskit finds it by name, or point `command` at any executable.\n"
	);
	let files = [
		("Cargo.toml", manifest),
		("src/main.rs", TOOL_PLUGIN.replace("__PLUGIN_NAME__", name)),
		("readme.md", readme),
		(".gitignore", "/target\n".to_owned()),
	];
	let dir = scaffold(destination, &files)?;
	println!("created {program} in {dir}");
	println!("next: cargo install --path {dir} && leptoskit plugin:{name} hello");
	Ok(())
}

/// Writes `files` under a freshly created `destination`, refusing any file
/// that already exists.
fn scaffold(destination: &std::path::Path, files: &[(&str, String)]) -> Result<Utf8PathBuf> {
	std::fs::create_dir_all(destination).context("create plugin directory")?;
	let dir = Utf8PathBuf::from_path_buf(std::fs::canonicalize(destination)?)
		.map_err(|_| anyhow::anyhow!("destination is not UTF-8"))?;
	for (relative, _) in files {
		let target = dir.join(relative);
		match std::fs::symlink_metadata(&target) {
			Ok(_) => anyhow::bail!("{target} already exists"),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
			Err(error) => return Err(error).with_context(|| format!("inspect {target}")),
		}
	}
	// Refuse an existing symlinked source directory instead of writing outside
	// the selected project via a pre-existing directory link.
	match std::fs::symlink_metadata(dir.join("src")) {
		Ok(metadata) => {
			anyhow::ensure!(
				metadata.is_dir() && !metadata.file_type().is_symlink(),
				"src must be a directory, not a symlink"
			);
		}
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
		Err(error) => return Err(error).context("inspect plugin source directory"),
	}
	for (relative, content) in files {
		write_new(&dir.join(relative), content)?;
	}
	Ok(dir)
}

const PLUGIN: &str = r#"//! __PLUGIN_NAME__ runtime plugin.

use leptoskit_bridge::{Bytes, Error, MethodId};

/// Raw bytes in, the same owned bytes out.
pub const ECHO: MethodId = MethodId(0);

/// A short, nonblocking Rust handler. Native asynchronous work should be
/// dispatched by the host and completed through `Bridge::complete`.
#[derive(Debug, Default)]
pub struct Plugin;

impl leptoskit_bridge::Plugin for Plugin {
    fn name(&self) -> &'static str { "__PLUGIN_NAME__" }
    fn methods(&self) -> &'static [MethodId] { &[ECHO] }
    fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
        match method {
            ECHO => Ok(payload),
            _ => Err(Error::Unavailable),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use leptoskit_bridge::Plugin as _;

    #[test]
    fn echo_preserves_the_payload_allocation() {
        let bytes = Bytes::from(vec![42; 4096]);
        let pointer = bytes.as_ptr();
        let result = Plugin.invoke(ECHO, bytes).unwrap();
        assert_eq!(result.as_ptr(), pointer);
        assert_eq!(result.as_ref(), &[42; 4096]);
        assert_eq!(Plugin.invoke(MethodId(99), Bytes::new()), Err(Error::Unavailable));
    }
}
"#;

/// The tooling plugin template: one process per hook, JSON in and out.
const TOOL_PLUGIN: &str = r#"//! A leptoskit tooling plugin.
//!
//! leptoskit runs this program once per hook with a JSON envelope on standard
//! input and expects one JSON object on standard output. Reply `ok: false`
//! with a `message` to fail the hook; anything else leptoskit reads from the
//! reply depends on the hook (`written` for generate, `features` and `env`
//! for the build hooks, `exit` for commands).

use std::io::Read as _;

fn main() {
	let mut envelope = String::new();
	std::io::stdin()
		.read_to_string(&mut envelope)
		.expect("read the envelope");
	let envelope: serde_json::Value = serde_json::from_str(&envelope).expect("a JSON envelope");
	let hook = envelope["hook"].as_str().unwrap_or_default();
	let context = &envelope["context"];
	let reply = match hook {
		"describe" => serde_json::json!({
			"ok": true,
			"protocol": 1,
			"cli_namespace": "__PLUGIN_NAME__",
			"commands": [
				{ "name": "hello", "about": "Greets the project", "flags": [] },
			],
		}),
		"generate" => serde_json::json!({
			"ok": true,
			// Files written during code generation; the dev watcher ignores
			// them, so rewriting never triggers another build.
			"written": [],
		}),
		"before_build" | "before_serve" => serde_json::json!({
			"ok": true,
			// "features": ["my-feature"],
			// "env": { "MY_FLAG": "1" },
			// "rustflags": [],
		}),
		"after_build" => serde_json::json!({ "ok": true }),
		"command" => serde_json::json!({
			"ok": true,
			"exit": 0,
			"message": format!("hello from __PLUGIN_NAME__ on {}", context["platform"]),
		}),
		other => serde_json::json!({
			"ok": false,
			"message": format!("unknown hook {other}"),
		}),
	};
	println!("{reply}");
}
"#;
