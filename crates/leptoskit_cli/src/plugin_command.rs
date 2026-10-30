//! `plugin:<namespace> <command>`: dispatch into the project's active
//! plugins.
//!
//! The registry resolves from `leptoskit.toml`, so the commands a project
//! answers follow its configuration: `leptoskit plugin:router routes` writes
//! the typed `Route` module whenever the router plugin is active.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::LeptoskitConfig;
use leptoskit_plugin::PluginRegistry;

/// The prefix every plugin command lives under.
const PREFIX: &str = "plugin:";

/// Whether one argument opens a plugin command, like `plugin:router`.
pub fn matches(argument: &str) -> bool {
	argument
		.strip_prefix(PREFIX)
		.is_some_and(|namespace| !namespace.is_empty())
}

/// Runs one plugin command from the raw arguments after the binary name.
///
/// # Errors
///
/// Returns an error outside a project, for an unknown namespace or command,
/// and for whatever the plugin itself reports.
pub fn run(arguments: &[String]) -> Result<()> {
	let namespace = arguments
		.first()
		.map(String::as_str)
		.map(|argument| &argument[PREFIX.len()..])
		.context("a plugin command needs a namespace, like `plugin:router routes`")?;
	let rest = &arguments[1..];
	let (config, root) = LeptoskitConfig::load()
		.with_context(|| format!("`plugin:{namespace}` runs inside a leptoskit project"))?;
	let registry = crate::build::resolve_plugins(&config)?;

	if rest.is_empty()
		|| rest
			.first()
			.is_some_and(|argument| argument == "--help" || argument == "-h")
	{
		print_help(&registry, namespace);
		return Ok(());
	}

	let command = &rest[0];
	let code = registry
		.run_command(&root, &config, namespace, command, &rest[1..])
		.map_err(|error| anyhow::anyhow!(error))?;
	if code != 0 {
		std::process::exit(code);
	}
	Ok(())
}

/// Lists the commands one namespace answers, or every namespace's.
fn print_help(registry: &PluginRegistry, namespace: &str) {
	if let Some(plugin) = registry.by_namespace(namespace) {
		println!("plugin:{namespace} commands:");
		for command in plugin.commands() {
			println!(
				"  plugin:{namespace} {:<12} {}",
				command.name, command.about
			);
			for flag in command.flags {
				println!("    --{}  {}", flag.name, flag.help);
			}
		}
		return;
	}
	crate::ui::warning(format_args!(
		"no plugin with the namespace `{namespace}` is active in this project"
	));
}

/// The `Plugin commands` section `leptoskit --help` appends when a project
/// declares plugins with namespaces; [`None`] outside a project or when none
/// do.
pub fn help_section(root: &Path) -> Option<String> {
	let config = LeptoskitConfig::load_from(root).ok()?;
	let registry = crate::build::resolve_plugins(&config).ok()?;
	let namespaced = registry.namespaced();
	if namespaced.is_empty() {
		return None;
	}
	let mut section = String::from("Plugin commands\n");
	for (namespace, commands) in namespaced {
		for command in commands {
			use std::fmt::Write as _;
			let _ = writeln!(
				section,
				"  plugin:{namespace} {:<12} {}",
				command.name, command.about
			);
		}
	}
	Some(section)
}
