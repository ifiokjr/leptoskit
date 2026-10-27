//! Plugins that are separate programs.
//!
//! A `[[plugins]]` entry with a `command` (or an executable named
//! `leptoskit-plugin-<name>` on PATH) runs as a child process: each hook is
//! one invocation, with a versioned JSON envelope on standard input and a
//! JSON reply on standard output. Any language works, nothing is linked, and
//! the protocol is the only contract.
//!
//! The envelope for every hook carries the protocol version, the plugin
//! name, the hook, and the hook's context:
//!
//! ```json
//! {"protocol": 1, "plugin": "tailwind-check", "hook": "generate",
//!  "context": {"root": "/abs/project", "platform": "web", "release": false,
//!              "options": {}}}
//! ```
//!
//! The reply is one JSON object. `ok: false` fails the hook with `message`;
//! `generate` replies with the files it wrote as `written`, the build hooks
//! reply with extra `features`, `env`, and `rustflags`, `command` replies
//! with the process exit code, and `describe` — asked once when the plugin
//! resolves — replies with the plugin's CLI namespace, supported platforms,
//! and commands.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command as StdCommand;
use std::process::Stdio;

use leptoskit_config::Platform;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;

use crate::LeptoskitPlugin;
use crate::PluginBuildContext;
use crate::PluginCommand;
use crate::PluginCommandContext;
use crate::PluginError;
use crate::PluginFlag;
use crate::PluginGenerateContext;
use crate::PluginMetadata;
use crate::PluginPostBuildContext;

/// The protocol version this host speaks.
pub const PROTOCOL: u32 = 1;

/// The name of the executable a plugin without an explicit `command` is
/// looked up as on PATH: `leptoskit-plugin-<name>`.
pub fn default_command(name: &str) -> String {
	format!("leptoskit-plugin-{name}")
}

/// Whether `command` names an executable on PATH.
pub fn command_exists(command: &str) -> bool {
	let Ok(path) = std::env::var("PATH") else {
		return false;
	};
	std::env::split_paths(&path).any(|directory| {
		let candidate = directory.join(command);
		let executable = std::fs::metadata(&candidate).is_ok_and(|metadata| metadata.is_file());
		#[cfg(unix)]
		let executable = executable && {
			use std::os::unix::fs::PermissionsExt as _;
			std::fs::metadata(&candidate)
				.is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
		};
		executable
	})
}

/// One hook call's envelope, as the child reads it on standard input.
#[derive(Serialize)]
struct Envelope<'a> {
	protocol: u32,
	plugin: &'a str,
	hook: &'a str,
	#[serde(skip_serializing_if = "Value::is_null")]
	context: Value,
}

/// One hook call's reply, as the child writes it on standard output.
#[derive(Deserialize, Default)]
struct Reply {
	#[serde(default)]
	ok: bool,
	#[serde(default)]
	message: String,
	#[serde(default)]
	written: Vec<PathBuf>,
	#[serde(default)]
	features: Vec<String>,
	#[serde(default)]
	env: BTreeMap<String, String>,
	#[serde(default)]
	rustflags: Vec<String>,
	#[serde(default)]
	exit: i32,
	#[serde(default)]
	cli_namespace: Option<String>,
	#[serde(default)]
	platforms: Vec<Platform>,
	#[serde(default)]
	commands: Vec<ReplyCommand>,
}

/// One command of a `describe` reply.
#[derive(Deserialize)]
struct ReplyCommand {
	name: String,
	#[serde(default)]
	about: String,
	#[serde(default)]
	flags: Vec<ReplyFlag>,
}

/// One flag of a `describe` reply.
#[derive(Deserialize)]
struct ReplyFlag {
	name: String,
	#[serde(default)]
	help: String,
}

/// A plugin that is a separate program, spoken to over the process protocol.
#[derive(Debug)]
pub struct ProcessPlugin {
	name: String,
	command: String,
	options: Value,
	metadata: PluginMetadata,
	commands: Vec<PluginCommand>,
}

impl ProcessPlugin {
	/// Probes a plugin program: asks it to `describe` itself, validating the
	/// protocol version, and returns the adapter for it.
	///
	/// # Errors
	///
	/// Returns an error when the program cannot run, answers with a foreign
	/// protocol version, or replies with anything but valid JSON.
	pub fn probe(name: &str, command: &str, options: Value) -> Result<Self, PluginError> {
		let plugin = Self {
			name: name.to_owned(),
			command: command.to_owned(),
			options,
			metadata: PluginMetadata::default(),
			commands: Vec::new(),
		};
		let reply = plugin.call("describe", json!({}))?;
		Ok(Self {
			metadata: PluginMetadata {
				cli_namespace: reply.cli_namespace,
				platforms: reply.platforms,
			},
			commands: reply
				.commands
				.into_iter()
				.map(|command| {
					PluginCommand {
						name: command.name,
						about: command.about,
						flags: command
							.flags
							.into_iter()
							.map(|flag| {
								PluginFlag {
									name: flag.name,
									help: flag.help,
								}
							})
							.collect(),
					}
				})
				.collect(),
			..plugin
		})
	}

	/// Runs one hook, returning its reply.
	fn call(&self, hook: &str, context: Value) -> Result<Reply, PluginError> {
		let envelope = Envelope {
			protocol: PROTOCOL,
			plugin: &self.name,
			hook,
			context,
		};
		let mut child = StdCommand::new(&self.command)
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.map_err(|error| self.error(format!("could not run {}: {error}", self.command)))?;
		let envelope =
			serde_json::to_vec(&envelope).map_err(|error| self.error(error.to_string()))?;
		child
			.stdin
			.as_mut()
			.expect("stdin is piped")
			.write_all(&envelope)
			.map_err(|error| self.error(format!("could not write the envelope: {error}")))?;
		let output = child
			.wait_with_output()
			.map_err(|error| self.error(error.to_string()))?;
		if !output.status.success() {
			return Err(self.error(format!(
				"exited with {}: {}",
				output.status,
				String::from_utf8_lossy(&output.stderr).trim()
			)));
		}
		let reply: Reply = serde_json::from_slice(&output.stdout)
			.map_err(|error| self.error(format!("reply is not valid JSON: {error}")))?;
		if !reply.ok {
			return Err(self.error(if reply.message.is_empty() {
				"replied not ok without a message".to_owned()
			} else {
				reply.message
			}));
		}
		Ok(reply)
	}

	fn error(&self, message: String) -> PluginError {
		PluginError::Hook {
			plugin: self.name.clone(),
			message,
		}
	}

	fn context(&self, root: &Path, platform: Platform, release: bool) -> Value {
		json!({
			"root": root.display().to_string(),
			"platform": platform.to_string(),
			"release": release,
			"options": self.options,
		})
	}

	/// Applies a build hook's reply to the context it answered for.
	fn apply(reply: &Reply, context: &mut PluginBuildContext) {
		for feature in &reply.features {
			context.add_feature(feature.clone());
		}
		for (key, value) in &reply.env {
			context.set_env(key.clone(), value.clone());
		}
		context.rustflags.extend(reply.rustflags.iter().cloned());
	}
}

impl LeptoskitPlugin for ProcessPlugin {
	fn name(&self) -> &str {
		&self.name
	}

	fn metadata(&self) -> PluginMetadata {
		self.metadata.clone()
	}

	fn commands(&self) -> Vec<PluginCommand> {
		self.commands.clone()
	}

	fn run_command(
		&self,
		context: &PluginCommandContext,
		command: &str,
		args: &[String],
	) -> Result<i32, PluginError> {
		let reply = self.call(
			"command",
			json!({
				"root": context.root.display().to_string(),
				"options": self.options,
				"command": command,
				"args": args,
			}),
		)?;
		if !reply.message.is_empty() {
			println!("[{}] {}", self.name, reply.message);
		}
		Ok(reply.exit)
	}

	fn generate(&self, context: &mut PluginGenerateContext) -> Result<(), PluginError> {
		let reply = self.call(
			"generate",
			self.context(&context.root, context.platform, context.release),
		)?;
		for written in &reply.written {
			context.generated(context.root.join(written));
		}
		Ok(())
	}

	fn before_build(&self, context: &mut PluginBuildContext) -> Result<(), PluginError> {
		let reply = self.call(
			"before_build",
			json!({
				"platform": context.platform.to_string(),
				"release": context.release,
				"options": self.options,
			}),
		)?;
		Self::apply(&reply, context);
		Ok(())
	}

	fn after_build(&self, context: &mut PluginPostBuildContext) -> Result<(), PluginError> {
		self.call(
			"after_build",
			json!({
				"platform": context.platform.to_string(),
				"release": context.release,
				"output_dir": context.output_dir.display().to_string(),
				"wasm_bundle": context.wasm_bundle
					.as_ref()
					.map(|path| path.display().to_string()),
				"options": self.options,
			}),
		)?;
		Ok(())
	}

	fn before_serve(&self, context: &PluginBuildContext) -> Result<(), PluginError> {
		// The serve context is immutable by contract, so this hook runs for
		// its side effects alone, like starting a companion process.
		self.call(
			"before_serve",
			json!({
				"platform": context.platform.to_string(),
				"release": context.release,
				"options": self.options,
			}),
		)?;
		Ok(())
	}
}

#[cfg(all(test, unix))]
mod tests {
	use leptoskit_config::LeptoskitConfig;

	use super::*;

	/// A plugin program answering every hook from a shell script.
	fn script_plugin(tag: &str) -> (String, String) {
		let script = format!(
			"#!/bin/sh\nINPUT=\"$(cat)\"\ncase \"$INPUT\" in\n*'\"describe\"'*) echo \
			 '{{\"ok\":true,\"protocol\":1,\"cli_namespace\":\"{tag}\",\"commands\":[{{\"name\":\"\
			 hi\",\"about\":\"Says hi\"}}]}}' ;;\n*'\"generate\"'*) echo \
			 '{{\"ok\":true,\"protocol\":1,\"written\":[\"src/gen.rs\"]}}' ;;\n*'\"command\"'*) \
			 echo '{{\"ok\":true,\"protocol\":1,\"exit\":7,\"message\":\"checked\"}}' \
			 ;;\n*'\"before_build\"'*) echo \
			 '{{\"ok\":true,\"protocol\":1,\"features\":[\"extra\"],\"env\":{{\"DEMO\":\"1\"}}}}' \
			 ;;\n*) echo '{{\"ok\":false,\"message\":\"unknown hook\"}}' ;;\nesac\n"
		);
		let path =
			std::env::temp_dir().join(format!("leptoskit-plugin-{tag}-{}.sh", std::process::id()));
		std::fs::write(&path, script).expect("write the plugin script");
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
			.expect("make the plugin executable");
		(tag.to_owned(), path.display().to_string())
	}

	#[test]
	fn probing_reads_the_plugins_surface() {
		let (tag, command) = script_plugin("probe");
		let plugin = ProcessPlugin::probe("demo", &command, serde_json::json!({ "strict": true }))
			.expect("the plugin describes itself");
		assert_eq!(
			plugin.metadata().cli_namespace.as_deref(),
			Some(tag.as_str())
		);
		let commands = plugin.commands();
		assert_eq!(commands.len(), 1);
		assert_eq!(commands[0].name, "hi");
	}

	#[test]
	fn generate_records_the_written_files() {
		let (_, command) = script_plugin("generate");
		let plugin = ProcessPlugin::probe("demo", &command, Value::Null).expect("probe");
		let root = std::env::temp_dir().join(format!("leptoskit-process-{}", std::process::id()));
		std::fs::create_dir_all(&root).expect("root");
		let mut context = PluginGenerateContext::new(&root, Platform::Web, false);
		plugin.generate(&mut context).expect("the hook runs");
		assert_eq!(context.outputs(), [root.join("src/gen.rs")]);
	}

	#[test]
	fn before_build_applies_the_reply() {
		let (_, command) = script_plugin("build");
		let plugin = ProcessPlugin::probe("demo", &command, Value::Null).expect("probe");
		let mut context = PluginBuildContext::new(LeptoskitConfig::default(), Platform::Web, false);
		plugin.before_build(&mut context).expect("the hook runs");
		assert!(context.cargo_features.contains(&"extra".to_owned()));
		assert_eq!(context.env.get("DEMO").map(String::as_str), Some("1"));
	}

	#[test]
	fn commands_return_the_replys_exit_code() {
		let (_, command) = script_plugin("command");
		let plugin = ProcessPlugin::probe("demo", &command, Value::Null).expect("probe");
		let context = PluginCommandContext {
			root: std::env::temp_dir(),
			config: LeptoskitConfig::default(),
		};
		let code = plugin
			.run_command(&context, "hi", &[])
			.expect("the command runs");
		assert_eq!(code, 7);
	}

	#[test]
	fn broken_replies_name_the_plugin() {
		let path =
			std::env::temp_dir().join(format!("leptoskit-plugin-broken-{}.sh", std::process::id()));
		std::fs::write(&path, "#!/bin/sh\ncat > /dev/null\necho 'not json'\n").expect("write");
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
		let error = ProcessPlugin::probe("broken", &path.display().to_string(), Value::Null)
			.expect_err("the reply is not JSON");
		assert!(error.to_string().contains("broken"), "{error}");
		assert!(error.to_string().contains("not valid JSON"), "{error}");
	}

	#[test]
	fn default_commands_are_discovered_on_path() {
		assert_eq!(default_command("tailwind"), "leptoskit-plugin-tailwind");
		assert!(command_exists("sh"));
		assert!(!command_exists("leptoskit-plugin-never-installed"));
	}
}
